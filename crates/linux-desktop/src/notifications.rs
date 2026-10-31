//! Desktop notifications over `org.freedesktop.Notifications`, on one long-lived connection
//! so that a click can come back as `ActionInvoked`.
use std::collections::HashMap;

use zbus::{blocking::Connection, proxy, zvariant::Value};

use crate::{Bus, Unavailable, APPLICATION_ID};

pub struct Alert<'a> {
    pub title: &'a str,
    pub body: &'a str,
}

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    gen_async = false
)]
trait Desktop {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

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

    pub fn send(&self, alert: &Alert<'_>) -> Result<u32, Unavailable> {
        let hints = HashMap::from([("desktop-entry", Value::from(APPLICATION_ID))]);
        self.proxy()?
            .notify(
                "Ezerbot",
                0,
                "",
                alert.title,
                alert.body,
                &["default", "Ouvrir"],
                hints,
                -1,
            )
            .map_err(|_| Unavailable)
    }

    /// Calls `handler` with the id of each notification of ours that was clicked. The thread
    /// ends with the connection, which closes when this `Notifier` is dropped.
    pub fn on_click(
        &self,
        mut handler: impl FnMut(u32) + Send + 'static,
    ) -> Result<(), Unavailable> {
        let proxy = self.proxy()?;
        let clicks = proxy.receive_action_invoked().map_err(|_| Unavailable)?;
        std::thread::spawn(move || {
            let _proxy = proxy;
            for click in clicks {
                if let Ok(args) = click.args() {
                    if args.action_key == "default" {
                        handler(args.id);
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
