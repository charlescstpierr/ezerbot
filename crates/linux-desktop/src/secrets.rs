//! Byte-level client of the Secret Service. A vault's items are found wherever they are, not
//! only in the collection `default` points at now. A write replaces the account's item where it
//! is; a new account goes where the others already live. An account found twice is ambiguous and
//! makes reads and writes `Unavailable`. It never unlocks: a locked collection that holds any of
//! them is `Unavailable`. A service may still ask the user to confirm a write or a deletion, and
//! `secret-service` then waits for the answer with no limit, so these two operations are bounded
//! and report `Unavailable` when no answer comes.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use secret_service::{
    blocking::{Collection as Native, Item, SecretService},
    EncryptionType,
};
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
    /// Checks that the collection that holds, or would hold, this vault's items is unlocked.
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
        collection.with_home(None, |_| Ok(()))?;
        Ok(collection)
    }

    pub fn entries(&self) -> Result<Vec<(String, Vec<u8>)>, Unavailable> {
        let entries: Vec<(String, Vec<u8>)> = self
            .found(self.scope_attributes())?
            .iter()
            .map(|item| {
                let account = item.get_attributes()?.remove("account");
                Ok(account.zip(Some(item.get_secret()?)))
            })
            .filter_map(Result::transpose)
            .collect::<Result<_, secret_service::Error>>()
            .map_err(|_| Unavailable)?;
        let accounts: HashSet<&str> = entries
            .iter()
            .map(|(account, _)| account.as_str())
            .collect();
        match accounts.len() == entries.len() {
            true => Ok(entries),
            false => Err(Unavailable),
        }
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
            this.with_home(Some(&account), |collection| {
                let attributes = this.attributes(&account);
                collection
                    .create_item(&label, attributes, &bytes, true, CONTENT_TYPE)
                    .map(drop)
            })?;
            match this.found(this.attributes(&account))?.as_slice() {
                [item] if item.get_secret().map_err(|_| Unavailable)? == bytes => Ok(()),
                _ => Err(Unavailable),
            }
        })
    }

    /// A missing item is a success; an item that is still found afterwards is not.
    pub fn delete_verified(&self, account: &str) -> Result<(), Unavailable> {
        let this = self.clone();
        let account = account.to_owned();
        self.confirmed(move || {
            for item in this.found(this.attributes(&account))? {
                item.delete().map_err(|_| Unavailable)?;
            }
            match this.found(this.attributes(&account))?.is_empty() {
                true => Ok(()),
                false => Err(Unavailable),
            }
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

    fn scope_attributes(&self) -> HashMap<&str, &str> {
        HashMap::from([
            ("application", APPLICATION),
            ("service", self.scope.as_str()),
        ])
    }

    fn attributes<'a>(&'a self, account: &'a str) -> HashMap<&'a str, &'a str> {
        let mut attributes = self.scope_attributes();
        attributes.insert("account", account);
        attributes
    }

    /// Matching items in every collection. One in a locked collection makes any read partial,
    /// so nothing is returned rather than the unlocked rest.
    fn found(&self, attributes: HashMap<&str, &str>) -> Result<Vec<Item<'_>>, Unavailable> {
        let found = self
            .service
            .search_items(attributes)
            .map_err(|_| Unavailable)?;
        if !found.locked.is_empty() {
            return Err(Unavailable);
        }
        Ok(found.unlocked)
    }

    /// Where an account's item goes: the collection of its current item, else the one that holds
    /// this vault's other items, else `default`.
    fn with_home<T>(
        &self,
        account: Option<&str>,
        action: impl FnOnce(&Native<'_>) -> Result<T, secret_service::Error>,
    ) -> Result<T, Unavailable> {
        let mut held = match account {
            Some(account) => self.found(self.attributes(account))?,
            None => Vec::new(),
        };
        if held.len() > 1 {
            return Err(Unavailable);
        }
        if held.is_empty() {
            held = self.found(self.scope_attributes())?;
        }
        let collection = match held.first() {
            Some(item) => self
                .service
                .get_all_collections()
                .map_err(|_| Unavailable)?
                .into_iter()
                .find(|collection| {
                    item.item_path
                        .as_str()
                        .strip_prefix(collection.collection_path.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .ok_or(Unavailable)?,
            None => self
                .service
                .get_default_collection()
                .map_err(|_| Unavailable)?,
        };
        // Checked before every call so that a locked collection never reaches an unlock prompt.
        if collection.is_locked().map_err(|_| Unavailable)? {
            return Err(Unavailable);
        }
        action(&collection).map_err(|_| Unavailable)
    }
}
