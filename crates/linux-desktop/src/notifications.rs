//! Desktop notifications over `org.freedesktop.Notifications`, on one long-lived connection
//! so that a click can come back as `ActionInvoked`.
use std::collections::HashMap;

use zbus::{blocking::Connection, proxy, zvariant::Value};

use crate::{Bus, Unavailable, APPLICATION_ID};

pub struct Alert<'a> {
    pub title: &'a str,
    pub body: &'a str,
}

/// A notification as one server knows it. An id means nothing elsewhere: a restarted server
/// counts from 1 again, and that id may then belong to another application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    server: String,
    pub id: u32,
}

const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    gen_async = false
)]
trait Desktop {
    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
}

pub struct Notifier {
    connection: Connection,
}

impl Notifier {
    pub fn connect(bus: &Bus) -> Result<Self, Unavailable> {
        Ok(Self {
            connection: bus.connect()?,
        })
    }

    fn proxy(&self) -> Result<DesktopProxy<'static>, Unavailable> {
        DesktopProxy::new(&self.connection).map_err(|_| Unavailable)
    }

    pub fn is_ready(&self) -> bool {
        self.proxy()
            .is_ok_and(|proxy| proxy.get_capabilities().is_ok())
    }

    /// The unique bus name in the reply is the server that issued the id, even mid-restart.
    pub fn send(&self, alert: &Alert<'_>) -> Result<Shown, Unavailable> {
        let hints = HashMap::from([("desktop-entry", Value::from(APPLICATION_ID))]);
        let reply = self
            .proxy()?
            .inner()
            .call_method(
                "Notify",
                &(
                    "Ezerbot",
                    0u32,
                    "",
                    alert.title,
                    alert.body,
                    &["default", "Ouvrir"][..],
                    hints,
                    -1i32,
                ),
            )
            .map_err(|_| Unavailable)?;
        let header = reply.header();
        Ok(Shown {
            server: header.sender().ok_or(Unavailable)?.to_string(),
            id: reply.body().deserialize().map_err(|_| Unavailable)?,
        })
    }

    /// Removes a notification from the desktop. It goes to the server that showed it, so a
    /// later server can never close what another application holds under the same id. One
    /// that is already gone may be an error.
    pub fn close(&self, shown: &Shown) -> Result<(), Unavailable> {
        self.connection
            .call_method(
                Some(shown.server.as_str()),
                PATH,
                Some(INTERFACE),
                "CloseNotification",
                &shown.id,
            )
            .map(drop)
            .map_err(|_| Unavailable)
    }

    /// Calls `handler` with each notification that was clicked; the signal also reaches us for
    /// other applications. The thread ends with the connection, which closes when this
    /// `Notifier` is dropped.
    pub fn on_click(
        &self,
        mut handler: impl FnMut(Shown) + Send + 'static,
    ) -> Result<(), Unavailable> {
        let proxy = self.proxy()?;
        let clicks = proxy.receive_action_invoked().map_err(|_| Unavailable)?;
        std::thread::spawn(move || {
            let _proxy = proxy;
            for click in clicks {
                let header = click.message().header();
                if let (Ok(args), Some(server)) = (click.args(), header.sender()) {
                    if args.action_key == "default" {
                        handler(Shown {
                            server: server.to_string(),
                            id: args.id,
                        });
                    }
                }
            }
        });
        Ok(())
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        let _ = self.connection.clone().close();
    }
}
