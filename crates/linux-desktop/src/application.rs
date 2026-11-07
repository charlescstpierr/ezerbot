//! Single instance and link inbox on the standard `org.freedesktop.Application` interface.
//! The limits mirror `receiveDeepLink` in `app/native/main.m`: 2048 bytes, 16 pending.
use std::{
    collections::HashMap,
    ffi::OsString,
    sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
    time::Duration,
};

use zbus::{
    blocking::{Connection, Proxy},
    fdo,
    names::WellKnownName,
    zvariant::{OwnedValue, Value},
};

use crate::{Bus, Unavailable, APPLICATION_ID};

const PATH: &str = "/ca/ezerbot/Ezerbot";
const INTERFACE: &str = "org.freedesktop.Application";
const MAX_LINK_BYTES: usize = 2048;
const MAX_PENDING: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    Open(String),
    Activate,
}

pub enum Instance {
    Primary(Inbox),
    Secondary(Remote),
}

pub struct Inbox {
    receiver: Receiver<Received>,
    _connection: Connection,
}

pub struct Remote {
    connection: Connection,
}

fn is_link(text: &str) -> bool {
    text.len() <= MAX_LINK_BYTES
        && text
            .split_once("://")
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("ezerbot"))
}

/// The `ezerbot://` links among the arguments the system passed to the process.
pub fn deep_links(args: impl IntoIterator<Item = OsString>) -> Vec<String> {
    args.into_iter()
        .filter_map(|arg| arg.into_string().ok())
        .filter(|arg| is_link(arg))
        .take(MAX_PENDING)
        .collect()
}

struct Service(SyncSender<Received>);

impl Service {
    fn push(&self, received: Received) -> fdo::Result<()> {
        self.0.try_send(received).map_err(|error| match error {
            TrySendError::Full(_) => fdo::Error::LimitsExceeded("trop de liens en attente".into()),
            TrySendError::Disconnected(_) => fdo::Error::Failed("application fermée".into()),
        })
    }
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl Service {
    fn open(
        &self,
        uris: Vec<String>,
        _platform_data: HashMap<String, OwnedValue>,
    ) -> fdo::Result<()> {
        uris.into_iter()
            .filter(|uri| is_link(uri))
            .try_for_each(|uri| self.push(Received::Open(uri)))
    }

    fn activate(&self, _platform_data: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        self.push(Received::Activate)
    }

    fn activate_action(
        &self,
        _name: String,
        _parameter: Vec<OwnedValue>,
        _platform_data: HashMap<String, OwnedValue>,
    ) -> fdo::Result<()> {
        self.push(Received::Activate)
    }
}

impl Instance {
    /// Owns the application name when nobody does (the caller is then the instance that
    /// shows the window), otherwise reaches the owner. Never queues for the name.
    pub fn claim(bus: &Bus) -> Result<Self, Unavailable> {
        let connection = bus.connect()?;
        let (sender, receiver) = sync_channel(MAX_PENDING);
        connection
            .object_server()
            .at(PATH, Service(sender))
            .map_err(|_| Unavailable)?;
        let name = WellKnownName::try_from(APPLICATION_ID).map_err(|_| Unavailable)?;
        match connection
            .request_name_with_flags(name, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        {
            Ok(fdo::RequestNameReply::PrimaryOwner | fdo::RequestNameReply::AlreadyOwner) => {
                Ok(Self::Primary(Inbox {
                    receiver,
                    _connection: connection,
                }))
            }
            Ok(_) | Err(zbus::Error::NameTaken) => Ok(Self::Secondary(Remote { connection })),
            Err(_) => Err(Unavailable),
        }
    }
}

impl Inbox {
    pub fn try_recv(&self) -> Option<Received> {
        self.receiver.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<Received> {
        self.receiver.recv_timeout(timeout).ok()
    }
}

impl Remote {
    pub fn open(&self, links: &[String]) -> Result<(), Unavailable> {
        self.proxy()?
            .call_method("Open", &(links, no_platform_data()))
            .map(|_| ())
            .map_err(|_| Unavailable)
    }

    pub fn activate(&self) -> Result<(), Unavailable> {
        self.proxy()?
            .call_method("Activate", &(no_platform_data(),))
            .map(|_| ())
            .map_err(|_| Unavailable)
    }

    fn proxy(&self) -> Result<Proxy<'_>, Unavailable> {
        Proxy::new(&self.connection, APPLICATION_ID, PATH, INTERFACE).map_err(|_| Unavailable)
    }
}

fn no_platform_data() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}
