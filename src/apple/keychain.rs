//! The protected keychain the device's keys live in, under the app's access group.
//!
//! The app and the extension each install it in their own process, whichever runs first, so
//! both read the same entry from the group they share, and the extension finds it after first
//! unlock on a locked device.

use std::collections::HashMap;
use std::sync::Arc;

use keyring_core::api::CredentialStoreApi;
use keyring_core::{CredentialStore, Entry, Error as StoreError};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSBundle, NSString};

/// The `Info.plist` key of the keychain access group the app and the extension share.
const ACCESS_GROUP_KEY: &str = "PushupsKeychainAccessGroup";

/// The access policy of the keys' entry, which the extension reads after first unlock.
const ACCESS_POLICY: &str = "AfterFirstUnlockThisDeviceOnly";

/// The protected store with the crate's access policy on every entry it builds, since the
/// `KeyStore` builds its entries without modifiers.
struct PolicyStore(Arc<CredentialStore>);

impl CredentialStoreApi for PolicyStore {
    fn vendor(&self) -> String {
        self.0.vendor()
    }

    fn id(&self) -> String {
        self.0.id()
    }

    fn build(
        &self,
        service: &str,
        user: &str,
        modifiers: Option<&HashMap<&str, &str>>,
    ) -> Result<Entry, StoreError> {
        let mut modifiers = modifiers.cloned().unwrap_or_default();
        modifiers.insert("access-policy", ACCESS_POLICY);
        self.0.build(service, user, Some(&modifiers))
    }

    fn search(&self, spec: &HashMap<&str, &str>) -> Result<Vec<Entry>, StoreError> {
        self.0.search(spec)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self.0.as_any()
    }

    fn persistence(&self) -> keyring_core::api::CredentialPersistence {
        self.0.persistence()
    }

    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.debug_fmt(f)
    }
}

/// The keychain access group the main bundle declares, none when it declares none.
fn access_group() -> Option<String> {
    let info = NSBundle::mainBundle().infoDictionary()?;
    // SAFETY: `objectForKey:` accepts any object as a key and returns nil for an absent one.
    let key = NSString::from_str(ACCESS_GROUP_KEY);
    let value: Option<Retained<AnyObject>> = unsafe { msg_send![&*info, objectForKey: &*key] };
    value
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|string| string.to_string())
}

/// Installs the process-wide key store on the protected keychain, under the bundle's access
/// group, in whichever process runs first, the app or the extension.
pub(super) fn install() {
    let group = access_group();
    let mut config: HashMap<&str, &str> = HashMap::new();
    if let Some(group) = &group {
        config.insert("access-group", group.as_str());
    }
    let Ok(store) = apple_native_keyring_store::protected::Store::new_with_configuration(&config)
    else {
        return;
    };
    let store: Arc<CredentialStore> = store;
    crate::key_store::install(Arc::new(PolicyStore(store)), "rs.pushups");
}
