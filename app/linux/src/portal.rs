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
    time::Duration,
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
const BUS: &str = "org.freedesktop.DBus";
/// The portal may put a dialog in front of the user, so this is generous.
const RESPONSE_LIMIT: Duration = Duration::from_secs(120);
/// Binding a lost shortcut again stops after this many failed attempts.
const RESTORE_ATTEMPTS: u32 = 10;

/// What the page must hear about the shortcut the portal holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The portal ended it; Ezerbot is binding the same choice again.
    Lost,
    /// Bound again, or changed from the desktop: the portal's text for it.
    Bound(Option<String>),
    /// Every attempt to bind it again failed.
    Abandoned,
}

impl Notice {
    /// The call that tells the main page; the portal's text goes in as a JSON string.
    pub fn script(&self) -> String {
        match self {
            Notice::Lost => "window.EzerbotFloatingMain?.shortcutLost?.()".into(),
            Notice::Bound(trigger) => format!(
                "window.EzerbotFloatingMain?.shortcutBound?.({})",
                serde_json::Value::from(trigger.clone())
            ),
            Notice::Abandoned => "window.EzerbotFloatingMain?.shortcutAbandoned?.()".into(),
        }
    }
}

/// The wait before the next attempt to bind a lost shortcut again, doubling from
/// 250 ms up to 30 s; `None` once the attempts are spent.
fn retry_delay(failed: u32) -> Option<Duration> {
    (failed < RESTORE_ATTEMPTS).then(|| {
        Duration::from_millis(250u64.saturating_mul(1 << failed.min(16)))
            .min(Duration::from_secs(30))
    })
}

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
    /// The portal's free, localized text for what it assigned. Shown, never parsed.
    trigger: Option<String>,
}

#[derive(Default)]
struct State {
    connection: Option<DBusConnection>,
    chosen: Option<&'static str>,
    held: Option<Held>,
    binding: bool,
    requests: u32,
    /// A shortcut the portal ended, to bind again until it holds or the attempts run out.
    wanted: Option<&'static Shortcut>,
    failed: u32,
    retry: Option<glib::SourceId>,
}

impl State {
    /// A session the portal ended no longer binds anything, so the same choice
    /// must bind again. A `disabled` choice holds no session and stays.
    /// True when a shortcut in force was lost: the page must hear it, or it keeps
    /// announcing a shortcut that no longer opens anything.
    fn forget(&mut self) -> bool {
        let Some(held) = self.held.take() else {
            return false;
        };
        self.chosen = None;
        self.wanted = Some(held.shortcut);
        self.failed = 0;
        true
    }

    /// A choice of the page replaces any restoration in progress.
    fn stop_restoring(&mut self) {
        self.wanted = None;
        self.failed = 0;
        if let Some(retry) = self.retry.take() {
            retry.remove();
        }
    }
}

/// The new text of `name` in a `ShortcutsChanged` signal `(o a(sa{sv}))` for `session`.
fn changed_trigger(parameters: &Variant, session: &str, name: &str) -> Option<Option<String>> {
    let (path, list) = parameters.get::<(
        glib::variant::ObjectPath,
        Vec<(String, HashMap<String, Variant>)>,
    )>()?;
    if path.as_str() != session {
        return None;
    }
    let (_, details) = list.into_iter().find(|(id, _)| id == name)?;
    Some(
        details
            .get("trigger_description")
            .and_then(|text| text.str().map(str::to_owned))
            .filter(|text| !text.is_empty()),
    )
}

#[derive(Clone)]
pub struct Portal {
    state: Rc<RefCell<State>>,
    activated: Rc<dyn Fn(Option<String>)>,
    notice: Rc<dyn Fn(Notice)>,
}

impl Portal {
    /// `activated` receives the portal's `activation_token`, the only thing that lets
    /// a Wayland compositor hand the keyboard to the panel.
    /// `notice` hears when the portal ends the shortcut in force on its own, when
    /// Ezerbot binds it again, and when the desktop changes its combination.
    pub fn new(
        activated: impl Fn(Option<String>) + 'static,
        notice: impl Fn(Notice) + 'static,
    ) -> Self {
        Self {
            state: Rc::default(),
            activated: Rc::new(activated),
            notice: Rc::new(notice),
        }
    }

    /// The portal ended the shortcut in force: tell the page, then bind it again.
    fn lost(&self) {
        (self.notice)(Notice::Lost);
        self.restore();
    }

    /// One attempt to bind the lost shortcut again; a failure waits longer each
    /// time, and a portal that comes back starts a new series at once.
    fn restore(&self) {
        let next = {
            let mut state = self.state.borrow_mut();
            state.retry = None;
            if state.binding || state.held.is_some() {
                return;
            }
            let Some(next) = state.wanted else { return };
            state.binding = true;
            next
        };
        let portal = self.clone();
        glib::spawn_future_local(async move {
            let result = portal.bind(next).await;
            let notice = {
                let mut state = portal.state.borrow_mut();
                state.binding = false;
                if state.wanted != Some(next) {
                    None
                } else if let Ok(trigger) = result {
                    state.stop_restoring();
                    Some(Notice::Bound(trigger))
                } else if result == Err(Outcome::Conflict) {
                    // The portal answered and refused: asking again changes nothing.
                    state.stop_restoring();
                    Some(Notice::Abandoned)
                } else {
                    state.failed += 1;
                    match retry_delay(state.failed - 1) {
                        Some(delay) => {
                            let again = portal.clone();
                            state.retry =
                                Some(glib::timeout_add_local_once(delay, move || again.restore()));
                            None
                        }
                        None => {
                            state.wanted = None;
                            Some(Notice::Abandoned)
                        }
                    }
                }
            };
            if let Some(notice) = notice {
                (portal.notice)(notice);
            }
        });
    }

    /// A replacement is bound in a new session, and the old session closed
    /// only once that succeeded, so no failure loses the shortcut in force.
    pub fn configure(&self, name: &str, done: impl FnOnce(Outcome, Option<String>) + 'static) {
        let step = {
            let state = self.state.borrow();
            plan(state.chosen, state.held.is_some(), state.binding, name)
        };
        match step {
            Plan::Answer(outcome) => {
                let trigger = match outcome {
                    Outcome::Registered => self
                        .state
                        .borrow()
                        .held
                        .as_ref()
                        .and_then(|held| held.trigger.clone()),
                    _ => None,
                };
                done(outcome, trigger);
            }
            Plan::Disable => {
                let (old, connection) = {
                    let mut state = self.state.borrow_mut();
                    state.stop_restoring();
                    state.chosen = Some(DISABLED);
                    (state.held.take(), state.connection.clone())
                };
                if let (Some(old), Some(connection)) = (old, connection) {
                    close(&connection, &old.session);
                }
                done(Outcome::Disabled, None);
            }
            Plan::Bind(next) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.stop_restoring();
                    state.binding = true;
                }
                let portal = self.clone();
                glib::spawn_future_local(async move {
                    let (outcome, trigger) = match portal.bind(next).await {
                        Ok(trigger) => (Outcome::Registered, trigger),
                        Err(outcome) => (outcome, None),
                    };
                    portal.state.borrow_mut().binding = false;
                    done(outcome, trigger);
                });
            }
        }
    }

    async fn bind(&self, next: &'static Shortcut) -> Result<Option<String>, Outcome> {
        let connection = self.connection().await?;
        let session = self.create_session(&connection).await?;
        let trigger = match self.bind_shortcut(&connection, &session, next).await {
            Ok(trigger) => trigger,
            Err(outcome) => {
                close(&connection, &session);
                return Err(outcome);
            }
        };
        let old = {
            let mut state = self.state.borrow_mut();
            state.chosen = Some(next.name);
            state.held.replace(Held {
                shortcut: next,
                session,
                trigger: trigger.clone(),
            })
        };
        if let Some(old) = old {
            close(&connection, &old.session);
        }
        Ok(trigger)
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
                    activated(activation_token(parameters));
                }
            },
        );
        // The desktop may change the combination at any time; the page shows the new one.
        let changed = self.clone();
        connection.signal_subscribe(
            Some(PORTAL),
            Some(SHORTCUTS),
            Some("ShortcutsChanged"),
            Some(PATH),
            None,
            DBusSignalFlags::NONE,
            move |_, _, _, _, _, parameters| {
                let trigger = {
                    let mut state = changed.state.borrow_mut();
                    let Some(held) = state.held.as_mut() else {
                        return;
                    };
                    let Some(trigger) =
                        changed_trigger(parameters, &held.session, held.shortcut.name)
                    else {
                        return;
                    };
                    held.trigger = trigger.clone();
                    trigger
                };
                (changed.notice)(Notice::Bound(trigger));
            },
        );
        // A session this client closed to replace it is no longer `held` when
        // its `Closed` arrives, so only the session in force is forgotten.
        let closed = self.clone();
        connection.signal_subscribe(
            Some(PORTAL),
            Some(SESSION),
            Some("Closed"),
            None,
            None,
            DBusSignalFlags::NONE,
            move |_, _, path, _, _, _| {
                let lost = {
                    let mut state = closed.state.borrow_mut();
                    state.held.as_ref().is_some_and(|held| held.session == path) && state.forget()
                };
                if lost {
                    closed.lost();
                }
            },
        );
        // The portal left: its sessions are gone. It came back: bind again at once.
        let owner = self.clone();
        connection.signal_subscribe(
            Some(BUS),
            Some(BUS),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            Some(PORTAL),
            DBusSignalFlags::NONE,
            move |_, _, _, _, _, parameters| {
                let lost = owner.state.borrow_mut().forget();
                if lost {
                    owner.lost();
                    return;
                }
                let back = parameters
                    .try_child_value(2)
                    .and_then(|name| name.str().map(|name| !name.is_empty()))
                    .unwrap_or(false);
                let restart = {
                    let mut state = owner.state.borrow_mut();
                    let waiting = back && state.wanted.is_some() && !state.binding;
                    if waiting {
                        state.failed = 0;
                        if let Some(retry) = state.retry.take() {
                            retry.remove();
                        }
                    }
                    waiting
                };
                if restart {
                    owner.restore();
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
    ) -> Result<Option<String>, Outcome> {
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
        let list = results
            .get("shortcuts")
            .and_then(|list| list.get::<Vec<(String, HashMap<String, Variant>)>>())
            .unwrap_or_default();
        let (_, assigned) = list
            .into_iter()
            .find(|(id, _)| id == shortcut.name)
            .ok_or(Outcome::Conflict)?;
        Ok(assigned
            .get("trigger_description")
            .and_then(|text| text.str().map(str::to_owned))
            .filter(|text| !text.is_empty()))
    }
}

/// Waits for a portal answer, up to `limit`. A request that never gets one
/// would otherwise leave the bind in flight and refuse every later change.
async fn answered<T>(limit: Duration, response: impl Future<Output = T>) -> Result<T, Outcome> {
    glib::future_with_timeout(limit, response)
        .await
        .map_err(|_| Outcome::Error)
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
        Some(answered(RESPONSE_LIMIT, response).await)
    } else {
        None
    };
    connection.signal_unsubscribe(subscription);
    // A portal or an interface that is absent cannot answer the call at all.
    let answer = answer.ok_or(Outcome::Unavailable)??;
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

/// `options.activation_token` of an `Activated` signal `(o s t a{sv})`, when it is a string.
fn activation_token(parameters: &Variant) -> Option<String> {
    (parameters.n_children() >= 4)
        .then(|| parameters.child_value(3))
        .and_then(|options| {
            glib::VariantDict::new(Some(&options)).lookup_value("activation_token", None)
        })
        .and_then(|token| token.str().map(str::to_owned))
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_activation_token_is_read_from_the_options_of_activated() {
        use gtk::glib::{variant::ObjectPath, ToVariant};
        use std::collections::HashMap;
        let signal = |options: &[(&str, Variant)]| {
            let options: HashMap<String, Variant> = options
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect();
            let session = ObjectPath::try_from("/org/freedesktop/portal/desktop/session/1/a");
            (session.unwrap(), "cmd+option+j", 7u64, options).to_variant()
        };
        let token = signal(&[("activation_token", "jeton-42".to_variant())]);
        assert_eq!(super::activation_token(&token).as_deref(), Some("jeton-42"));
        assert_eq!(super::activation_token(&signal(&[])), None);
        let wrong = signal(&[("activation_token", 7u32.to_variant())]);
        assert_eq!(super::activation_token(&wrong), None);
    }

    use super::*;

    #[test]
    fn a_wait_without_an_answer_ends_in_an_error() {
        let context = glib::MainContext::new();
        let silent = Delivery::<u32>::new();
        let limit = Duration::from_millis(20);
        assert_eq!(
            context.block_on(answered(limit, silent)),
            Err(Outcome::Error)
        );
        let answer = Delivery::new();
        answer.fulfil(7);
        assert_eq!(context.block_on(answered(limit, answer)), Ok(7));
    }

    #[test]
    fn forgetting_keeps_a_choice_that_held_no_session() {
        let mut state = State {
            chosen: Some(DISABLED),
            ..State::default()
        };
        state.forget();
        assert_eq!(state.chosen, Some(DISABLED));
        let shortcut = Shortcut::find("cmd+option+j").unwrap();
        state.chosen = Some(shortcut.name);
        state.held = Some(Held {
            shortcut,
            session: "/s".into(),
            trigger: None,
        });
        state.forget();
        assert!(state.held.is_none() && state.chosen.is_none());
        assert_eq!(state.wanted.map(|wanted| wanted.name), Some("cmd+option+j"));
    }

    #[test]
    fn a_lost_shortcut_is_retried_with_a_bounded_growing_wait() {
        let delays: Vec<_> = (0..RESTORE_ATTEMPTS)
            .map(|failed| retry_delay(failed).unwrap())
            .collect();
        assert_eq!(delays[0], Duration::from_millis(250));
        assert!(delays.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(*delays.last().unwrap(), Duration::from_secs(30));
        assert_eq!(retry_delay(RESTORE_ATTEMPTS), None);
        // About two minutes in all, then the page is told to choose again.
        assert!(delays.iter().sum::<Duration>() < Duration::from_secs(125));
    }

    #[test]
    fn the_page_hears_the_portal_text_as_a_json_string() {
        assert_eq!(
            Notice::Bound(Some("Ctrl+\"</script>".into())).script(),
            r#"window.EzerbotFloatingMain?.shortcutBound?.("Ctrl+\"</script>")"#
        );
        assert_eq!(
            Notice::Bound(None).script(),
            "window.EzerbotFloatingMain?.shortcutBound?.(null)"
        );
    }

    #[test]
    fn a_choice_of_the_page_stops_the_restoration() {
        let mut state = State {
            wanted: Shortcut::find("cmd+option+j"),
            failed: 3,
            ..State::default()
        };
        state.stop_restoring();
        assert!(state.wanted.is_none() && state.failed == 0 && state.retry.is_none());
    }

    #[test]
    fn shortcuts_changed_gives_the_new_text_of_the_session_in_force() {
        let signal = |session: &str, text: &str| {
            let details: HashMap<String, Variant> =
                HashMap::from([("trigger_description".to_string(), text.to_variant())]);
            let path = glib::variant::ObjectPath::try_from(session.to_owned()).unwrap();
            (path, vec![("cmd+option+j".to_string(), details)]).to_variant()
        };
        let ours = "/org/freedesktop/portal/desktop/session/1_1/ezerbot_session_1";
        assert_eq!(
            changed_trigger(&signal(ours, "Ctrl+Alt+K"), ours, "cmd+option+j"),
            Some(Some("Ctrl+Alt+K".into()))
        );
        assert_eq!(
            changed_trigger(&signal(ours, ""), ours, "cmd+option+j"),
            Some(None)
        );
        assert_eq!(
            changed_trigger(&signal("/other", "X"), ours, "cmd+option+j"),
            None
        );
        assert_eq!(
            changed_trigger(&signal(ours, "X"), ours, "ctrl+option+space"),
            None
        );
    }

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
