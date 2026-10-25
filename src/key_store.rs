//! The device's keys in the platform key store, one entry per service name.
//!
//! The entry's user is `keys`, and its secret is the 48 bytes `Keys::to_bytes` writes. The
//! process-wide store is installed once by the platform, before `register` or `rotate_keys`
//! runs, and `pushups` never sets `keyring_core`'s default store, so an app's own keyring use
//! is untouched.

use std::sync::{Arc, OnceLock};

use keyring_core::{CredentialStore, Entry, Error as StoreError};

use crate::error::Error;
use crate::webpush_crypto::Keys;

/// The entry's user.
const USER: &str = "keys";

static KEY_STORE: OnceLock<KeyStore> = OnceLock::new();

/// The device's keys in one credential store, under a service name and the `keys` user.
pub(crate) struct KeyStore {
    store: Arc<CredentialStore>,
    service: String,
}

impl core::fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KeyStore")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl KeyStore {
    /// New keys over `store`, under `service`.
    pub(crate) fn new(store: Arc<CredentialStore>, service: impl Into<String>) -> Self {
        Self {
            store,
            service: service.into(),
        }
    }

    /// The stored keys, or none when there is no entry or the entry does not hold a valid key
    /// pair.
    ///
    /// # Errors
    ///
    /// [`Error::Platform`] when the key store fails.
    pub(crate) fn load(&self) -> Result<Option<Keys>, Error> {
        let entry = self.entry()?;
        match entry.get_secret() {
            Ok(secret) => Ok(Keys::from_bytes(&secret)),
            Err(StoreError::NoEntry) => Ok(None),
            Err(error) => Err(store_error(&error)),
        }
    }

    /// The stored keys, creating and storing fresh ones when there is no entry or it does not
    /// hold a valid key pair.
    ///
    /// # Errors
    ///
    /// [`Error::Platform`] when the keys cannot be created or the key store fails.
    pub(crate) fn load_or_create(&self) -> Result<Keys, Error> {
        match self.load()? {
            Some(keys) => Ok(keys),
            None => self.store_new(),
        }
    }

    /// Fresh keys, replacing whatever the entry held.
    ///
    /// # Errors
    ///
    /// [`Error::Platform`] when the keys cannot be created or the key store fails.
    pub(crate) fn rotate(&self) -> Result<Keys, Error> {
        self.store_new()
    }

    /// The store's entry for the service and the `keys` user.
    ///
    /// # Errors
    ///
    /// [`Error::Platform`] when the key store fails.
    fn entry(&self) -> Result<Entry, Error> {
        self.store
            .build(&self.service, USER, None)
            .map_err(|error| store_error(&error))
    }

    /// Fresh keys, stored in the entry.
    ///
    /// # Errors
    ///
    /// [`Error::Platform`] when the keys cannot be created or the key store fails.
    fn store_new(&self) -> Result<Keys, Error> {
        let keys = Keys::generate().map_err(|_| randomness_error())?;
        self.entry()?
            .set_secret(&keys.to_bytes())
            .map_err(|error| store_error(&error))?;
        Ok(keys)
    }
}

/// Installs the process-wide key store. The first install wins.
pub(crate) fn install(store: Arc<CredentialStore>, service: impl Into<String>) {
    let _ = KEY_STORE.set(KeyStore::new(store, service));
}

/// The installed key store, or none when no platform installed one.
pub(crate) fn installed() -> Option<&'static KeyStore> {
    KEY_STORE.get()
}

/// A key store failure, named as such.
fn store_error(error: &StoreError) -> Error {
    Error::Platform(format!("the key store failed: {error}"))
}

/// A failed randomness source while creating fresh keys.
fn randomness_error() -> Error {
    Error::Platform("could not read random bytes for the key pair".to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use keyring_core::api::CredentialStoreApi;
    use keyring_core::mock::{Cred, Store};

    use super::*;

    fn key_store() -> (Arc<Store>, KeyStore) {
        let store = Store::new().expect("the mock store builds");
        (store.clone(), KeyStore::new(store.clone(), "rs.pushups"))
    }

    #[test]
    fn load_or_create_is_stable_across_calls_and_stores() {
        let (store, first) = key_store();
        let keys = first.load_or_create().expect("fresh keys store");
        let again = first.load_or_create().expect("the keys load");
        assert_eq!(again.to_bytes(), keys.to_bytes());
        let fresh = KeyStore::new(store.clone(), "rs.pushups");
        assert_eq!(
            fresh
                .load()
                .expect("the keys load")
                .expect("an entry now exists")
                .to_bytes(),
            keys.to_bytes()
        );
    }

    #[test]
    fn rotate_replaces_the_keys_and_persists_them() {
        let (store, first) = key_store();
        let old = first.load_or_create().expect("fresh keys store");
        let new = first.rotate().expect("fresh keys store");
        assert_ne!(new.to_bytes(), old.to_bytes());
        let fresh = KeyStore::new(store.clone(), "rs.pushups");
        assert_eq!(
            fresh
                .load()
                .expect("the keys load")
                .expect("the rotated keys persist")
                .to_bytes(),
            new.to_bytes()
        );
    }

    #[test]
    fn an_entry_with_unreadable_bytes_is_replaced() {
        let (raw, store) = key_store();
        let entry = raw
            .build("rs.pushups", "keys", None)
            .expect("the entry builds");
        entry
            .set_secret(&[0u8; 40])
            .expect("the wrong-length secret stores");
        let keys = store.load_or_create().expect("fresh keys store");
        assert_eq!(
            store
                .load()
                .expect("the keys load")
                .expect("the replacement loads")
                .to_bytes(),
            keys.to_bytes()
        );
    }

    #[test]
    fn a_store_error_surfaces() {
        let (_, keys) = key_store();
        let entry = keys.entry().expect("the entry builds");
        let cred = entry
            .as_any()
            .downcast_ref::<Cred>()
            .expect("the mock credential");
        cred.set_error(StoreError::PlatformFailure(
            "the key store locked out".into(),
        ));
        let error = keys.load().expect_err("the store failure surfaces");
        assert!(
            error.to_string().contains("the key store failed"),
            "the error names the key store"
        );
    }

    #[test]
    fn the_first_install_wins() {
        let (store, _) = key_store();
        install(store, "rs.pushups");
        let first = installed().expect("the store installs");
        assert_eq!(first.service, "rs.pushups");
        install(
            Store::new().expect("the mock store builds"),
            "rs.pushups.other",
        );
        let still = installed().expect("the first store installs");
        assert_eq!(still.service, "rs.pushups");
    }
}
