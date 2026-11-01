//! Byte-level client of the Secret Service default collection. It never unlocks: a locked
//! collection is `Unavailable`. A service may still ask the user to confirm a write or a
//! deletion, and `secret-service` then waits for the answer with no limit, so these two
//! operations are bounded and report `Unavailable` when no answer comes.
use std::{collections::HashMap, sync::Arc, time::Duration};

use secret_service::{blocking::SecretService, EncryptionType};
use zbus::blocking::Connection;

use crate::{bus::bounded, Bus, Unavailable};

const APPLICATION: &str = "ca.ezerbot.secrets.v1";
const CONTENT_TYPE: &str = "application/json";
const CONFIRMATION_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct Collection {
    connection: Connection,
    service: Arc<SecretService<'static>>,
    scope: String,
}

impl Collection {
    /// Checks that the default collection exists and is unlocked.
    pub fn open(bus: &Bus, scope: String) -> Result<Self, Unavailable> {
        let connection = bus.connect()?;
        let service = SecretService::connect_with_existing(EncryptionType::Dh, connection.clone())
            .map_err(|_| Unavailable)?;
        let service = Arc::new(service);
        let collection = Self {
            connection,
            service,
            scope,
        };
        collection.with_default(|_| Ok(()))?;
        Ok(collection)
    }

    pub fn entries(&self) -> Result<Vec<(String, Vec<u8>)>, Unavailable> {
        self.with_default(|collection| {
            let attributes =
                HashMap::from([("application", APPLICATION), ("service", &self.scope)]);
            collection
                .search_items(attributes)?
                .iter()
                .map(|item| {
                    let account = item.get_attributes()?.remove("account");
                    Ok(account.zip(Some(item.get_secret()?)))
                })
                .filter_map(Result::transpose)
                .collect()
        })
    }

    /// Replaces the item of this account, then reads it back and compares.
    pub fn put_verified(
        &self,
        account: &str,
        label: &str,
        bytes: &[u8],
    ) -> Result<(), Unavailable> {
        let this = self.clone();
        let (account, label, bytes) = (account.to_owned(), label.to_owned(), bytes.to_owned());
        self.confirmed(move || {
            this.with_default(|collection| {
                let attributes = this.attributes(&account);
                collection.create_item(&label, attributes, &bytes, true, CONTENT_TYPE)?;
                let items = collection.search_items(this.attributes(&account))?;
                match items.as_slice() {
                    [item] if item.get_secret()? == bytes => Ok(()),
                    _ => Err(secret_service::Error::NoResult),
                }
            })
        })
    }

    /// A missing item is a success; an item that is still found afterwards is not.
    pub fn delete_verified(&self, account: &str) -> Result<(), Unavailable> {
        let this = self.clone();
        let account = account.to_owned();
        self.confirmed(move || {
            this.with_default(|collection| {
                for item in collection.search_items(this.attributes(&account))? {
                    item.delete()?;
                }
                match collection
                    .search_items(this.attributes(&account))?
                    .is_empty()
                {
                    true => Ok(()),
                    false => Err(secret_service::Error::NoResult),
                }
            })
        })
    }

    /// A late confirmation must not replace what a newer write stored: giving up closes the
    /// connection, which ends the waiting thread and tells the service its caller is gone.
    fn confirmed(
        &self,
        work: impl FnOnce() -> Result<(), Unavailable> + Send + 'static,
    ) -> Result<(), Unavailable> {
        let connection = self.connection.clone();
        bounded(CONFIRMATION_LIMIT, work).inspect_err(|_| {
            std::thread::spawn(move || connection.close());
        })
    }

    fn attributes<'a>(&'a self, account: &'a str) -> HashMap<&'a str, &'a str> {
        HashMap::from([
            ("application", APPLICATION),
            ("service", &self.scope),
            ("account", account),
        ])
    }

    fn with_default<T>(
        &self,
        action: impl FnOnce(
            &secret_service::blocking::Collection<'_>,
        ) -> Result<T, secret_service::Error>,
    ) -> Result<T, Unavailable> {
        let collection = self
            .service
            .get_default_collection()
            .map_err(|_| Unavailable)?;
        // Checked before every call so that a locked collection never reaches an unlock prompt.
        if collection.is_locked().map_err(|_| Unavailable)? {
            return Err(Unavailable);
        }
        action(&collection).map_err(|_| Unavailable)
    }
}
