//! The GlobalShortcuts portal, for Wayland sessions where no client may grab a
//! key. Everything runs on the main context, so the state is not shared.
use crate::hotkey::{Outcome, Shortcut, DISABLED};
use gtk::gio::{self, prelude::*, DBusCallFlags, DBusConnection, DBusSignalFlags};
use gtk::glib::{self, Variant};
use std::{
    cell::RefCell,
    collections::HashMap,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";

#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Answer(Outcome),
    Disable,
    Bind(&'static Shortcut),
}

fn plan(chosen: Option<&str>, held: bool, binding: bool, name: &str) -> Plan {
    if binding {
        return Plan::Answer(Outcome::Error);
    }
    if chosen == Some(name) {
        return Plan::Answer(if held {
            Outcome::Registered
        } else {
            Outcome::Disabled
        });
    }
    if name == DISABLED {
        return Plan::Disable;
    }
    Shortcut::find(name).map_or(Plan::Answer(Outcome::Error), Plan::Bind)
}

/// A value a signal callback hands to the task waiting for it.
#[derive(Clone)]
struct Delivery<T>(Rc<RefCell<(Option<T>, Option<Waker>)>>);

impl<T> Delivery<T> {
    fn new() -> Self {
        Self(Rc::new(RefCell::new((None, None))))
    }

    fn fulfil(&self, value: T) {
        let mut inner = self.0.borrow_mut();
        inner.0 = Some(value);
        if let Some(waker) = inner.1.take() {
            waker.wake();
        }
    }
}

impl<T> Future for Delivery<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<T> {
        let mut inner = self.0.borrow_mut();
        match inner.0.take() {
            Some(value) => Poll::Ready(value),
            None => {
                inner.1 = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

struct Held {
    shortcut: &'static Shortcut,
    session: String,
}

#[derive(Default)]
struct State {
    connection: Option<DBusConnection>,
    chosen: Option<&'static str>,
    held: Option<Held>,
    binding: bool,
    requests: u32,
}

#[derive(Clone)]
pub struct Portal {
    state: Rc<RefCell<State>>,
    activated: Rc<dyn Fn()>,
}

impl Portal {
    pub fn new(activated: impl Fn() + 'static) -> Self {
        Self {
            state: Rc::default(),
            activated: Rc::new(activated),
        }
    }

    /// A replacement is bound in a new session, and the old session closed
    /// only once that succeeded, so no failure loses the shortcut in force.
    pub fn configure(&self, name: &str, done: impl FnOnce(Outcome) + 'static) {
        let step = {
            let state = self.state.borrow();
            plan(state.chosen, state.held.is_some(), state.binding, name)
        };
        match step {
            Plan::Answer(outcome) => done(outcome),
            Plan::Disable => {
                let (old, connection) = {
                    let mut state = self.state.borrow_mut();
                    state.chosen = Some(DISABLED);
                    (state.held.take(), state.connection.clone())
                };
                if let (Some(old), Some(connection)) = (old, connection) {
                    close(&connection, &old.session);
                }
                done(Outcome::Disabled);
            }
            Plan::Bind(next) => {
                self.state.borrow_mut().binding = true;
                let portal = self.clone();
                glib::spawn_future_local(async move {
                    let outcome = portal.bind(next).await;
                    portal.state.borrow_mut().binding = false;
                    done(outcome);
                });
            }
        }
    }

    async fn bind(&self, next: &'static Shortcut) -> Outcome {
        let connection = match self.connection().await {
            Ok(connection) => connection,
            Err(outcome) => return outcome,
        };
        let session = match self.create_session(&connection).await {
            Ok(session) => session,
            Err(outcome) => return outcome,
        };
        if let Err(outcome) = self.bind_shortcut(&connection, &session, next).await {
            close(&connection, &session);
            return outcome;
        }
        let old = {
            let mut state = self.state.borrow_mut();
            state.chosen = Some(next.name);
            state.held.replace(Held {
                shortcut: next,
                session,
            })
        };
        if let Some(old) = old {
            close(&connection, &old.session);
        }
        Outcome::Registered
    }

    async fn connection(&self) -> Result<DBusConnection, Outcome> {
        if let Some(connection) = self.state.borrow().connection.clone() {
            return Ok(connection);
        }
        let connection = gio::bus_get_future(gio::BusType::Session)
            .await
            .map_err(|_| Outcome::Unavailable)?;
        let state = Rc::clone(&self.state);
        let activated = Rc::clone(&self.activated);
        connection.signal_subscribe(
            Some(PORTAL),
            Some(SHORTCUTS),
            Some("Activated"),
            Some(PATH),
            None,
            DBusSignalFlags::NONE,
            move |_, _, _, _, _, parameters| {
                if parameters.n_children() < 2 {
                    return;
                }
                let (session, id) = (parameters.child_value(0), parameters.child_value(1));
                let ours = state.borrow().held.as_ref().is_some_and(|held| {
                    Some(held.session.as_str()) == session.str()
                        && Some(held.shortcut.name) == id.str()
                });
                if ours {
                    activated();
                }
            },
        );
        self.state.borrow_mut().connection = Some(connection.clone());
        Ok(connection)
    }

    fn token(&self) -> u32 {
        let mut state = self.state.borrow_mut();
        state.requests += 1;
        state.requests
    }

    async fn create_session(&self, connection: &DBusConnection) -> Result<String, Outcome> {
        let token = self.token();
        let options: HashMap<String, Variant> = HashMap::from([
            (
                "handle_token".into(),
                format!("ezerbot_{token}").to_variant(),
            ),
            (
                "session_handle_token".into(),
                format!("ezerbot_session_{token}").to_variant(),
            ),
        ]);
        let results = request(
            connection,
            "CreateSession",
            &format!("ezerbot_{token}"),
            |_| (options,).to_variant(),
        )
        .await?;
        results
            .get("session_handle")
            .and_then(|handle| handle.str().map(str::to_owned))
            .ok_or(Outcome::Error)
    }

    async fn bind_shortcut(
        &self,
        connection: &DBusConnection,
        session: &str,
        shortcut: &Shortcut,
    ) -> Result<(), Outcome> {
        let token = self.token();
        let session =
            glib::variant::ObjectPath::try_from(session.to_owned()).map_err(|_| Outcome::Error)?;
        let details: HashMap<String, Variant> = HashMap::from([
            (
                "description".into(),
                "Ouvrir le panneau flottant d’Ezerbot".to_variant(),
            ),
            ("preferred_trigger".into(), shortcut.trigger.to_variant()),
        ]);
        let options: HashMap<String, Variant> = HashMap::from([(
            "handle_token".into(),
            format!("ezerbot_{token}").to_variant(),
        )]);
        let shortcuts = vec![(shortcut.name.to_owned(), details)];
        let results = request(
            connection,
            "BindShortcuts",
            &format!("ezerbot_{token}"),
            |_| (session, shortcuts, String::new(), options).to_variant(),
        )
        .await?;
        let bound = results
            .get("shortcuts")
            .and_then(|list| list.get::<Vec<(String, HashMap<String, Variant>)>>())
            .is_some_and(|list| list.iter().any(|(id, _)| id == shortcut.name));
        if bound {
            Ok(())
        } else {
            Err(Outcome::Conflict)
        }
    }
}

/// Calls a portal method and waits for the `Response` on its request. The
/// subscription precedes the call: the reply may arrive after the signal.
async fn request(
    connection: &DBusConnection,
    method: &str,
    token: &str,
    parameters: impl FnOnce(&str) -> Variant,
) -> Result<HashMap<String, Variant>, Outcome> {
    let sender = connection
        .unique_name()
        .ok_or(Outcome::Unavailable)?
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("{PATH}/request/{sender}/{token}");
    let response = Delivery::new();
    let delivery = response.clone();
    let subscription = connection.signal_subscribe(
        Some(PORTAL),
        Some(REQUEST),
        Some("Response"),
        Some(&path),
        None,
        DBusSignalFlags::NONE,
        move |_, _, _, _, _, parameters| delivery.fulfil(parameters.clone()),
    );
    let reply = connection
        .call_future(
            Some(PORTAL),
            PATH,
            SHORTCUTS,
            method,
            Some(&parameters(token)),
            None,
            DBusCallFlags::NONE,
            -1,
        )
        .await;
    let answer = if reply.is_ok() {
        Some(response.await)
    } else {
        None
    };
    connection.signal_unsubscribe(subscription);
    // A portal or an interface that is absent cannot answer the call at all.
    let answer = answer.ok_or(Outcome::Unavailable)?;
    let (code, results) = answer
        .get::<(u32, HashMap<String, Variant>)>()
        .ok_or(Outcome::Error)?;
    if code == 0 {
        Ok(results)
    } else {
        Err(Outcome::Error)
    }
}

fn close(connection: &DBusConnection, session: &str) {
    let (connection, session) = (connection.clone(), session.to_owned());
    glib::spawn_future_local(async move {
        let _ = connection
            .call_future(
                Some(PORTAL),
                &session,
                SESSION,
                "Close",
                None,
                None,
                DBusCallFlags::NONE,
                -1,
            )
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bind_in_flight_refuses_every_other_request() {
        for name in ["cmd+option+j", DISABLED, "nonsense"] {
            assert_eq!(
                plan(None, false, true, name),
                Plan::Answer(Outcome::Error),
                "{name}"
            );
        }
    }

    #[test]
    fn the_chosen_name_answers_with_the_state_and_asks_nothing() {
        let held = Some("cmd+option+j");
        assert_eq!(
            plan(held, true, false, "cmd+option+j"),
            Plan::Answer(Outcome::Registered)
        );
        assert_eq!(
            plan(Some(DISABLED), false, false, DISABLED),
            Plan::Answer(Outcome::Disabled)
        );
    }

    #[test]
    fn other_names_disable_bind_or_are_errors() {
        assert_eq!(plan(None, false, false, DISABLED), Plan::Disable);
        assert_eq!(
            plan(Some("cmd+option+j"), true, false, DISABLED),
            Plan::Disable
        );
        let Plan::Bind(next) = plan(None, false, false, "ctrl+option+space") else {
            panic!("expected a bind");
        };
        assert_eq!(next.trigger, "CTRL+ALT+space");
        assert_eq!(plan(None, false, false, "x"), Plan::Answer(Outcome::Error));
    }
}
