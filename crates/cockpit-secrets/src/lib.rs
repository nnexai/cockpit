//! The OS vault behind [`CredentialVault`]: Linux Secret Service and macOS Keychain.
//!
//! Caching, timeouts and validation belong to `cockpit_core::credentials`; this
//! crate only maps the blocking vault calls onto `keyring-core`. Platform error
//! text is never kept: every failure other than "absent" is
//! [`VaultError::Unavailable`].
//!
//! Vault item identity (service `cockpit`, one account per provider instance)
//! is decided by the core; the label is passed to the store when it supports one.

use std::sync::Arc;

use cockpit_core::credentials::CredentialVault;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use cockpit_core::credentials::UnavailableVault;

/// The vault of the user session that runs the host process.
///
/// Other targets get a vault whose every call fails, so the CLI login applies.
pub fn os_vault() -> Arc<dyn CredentialVault> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        Arc::new(os::OsVault::new(os::platform_backend()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Arc::new(UnavailableVault)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod os {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use cockpit_core::credentials::{CredentialVault, VaultError};
    use keyring_core::{CredentialStore, Entry, Error};

    /// Every item is created under this service; the core owns the account.
    const SERVICE: &str = "cockpit";

    /// How to open the platform store.
    pub(super) struct Backend {
        open: fn() -> keyring_core::Result<Arc<CredentialStore>>,
        /// Whether the store accepts a `label` entry modifier.
        labels: bool,
    }

    #[cfg(target_os = "linux")]
    pub(super) fn platform_backend() -> Backend {
        Backend {
            open: || Ok(dbus_secret_service_keyring_store::Store::new()?),
            labels: true,
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn platform_backend() -> Backend {
        // The Keychain store rejects a `label` modifier, so items keep its default.
        Backend {
            open: || Ok(apple_native_keyring_store::keychain::Store::new()?),
            labels: false,
        }
    }

    /// The store is opened on first use, not at startup, and dropped after a
    /// failure: the session bus or the keyring daemon may appear or restart
    /// while Cockpit runs, and the next call must be able to recover.
    pub(super) struct OsVault {
        backend: Backend,
        store: Mutex<Option<Arc<CredentialStore>>>,
    }

    impl OsVault {
        pub(super) fn new(backend: Backend) -> Self {
            Self {
                backend,
                store: Mutex::new(None),
            }
        }

        fn store(&self) -> Result<Arc<CredentialStore>, VaultError> {
            let mut slot = self.store.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(store) = slot.as_ref() {
                return Ok(store.clone());
            }
            let store = (self.backend.open)().map_err(|_| VaultError::Unavailable)?;
            *slot = Some(store.clone());
            Ok(store)
        }

        /// Runs `operation` on the entry for `account`. A failure other than
        /// `NoEntry` discards the store unless another thread already replaced it.
        fn with_entry<T>(
            &self,
            account: &str,
            label: Option<&str>,
            operation: impl FnOnce(&Entry) -> keyring_core::Result<T>,
        ) -> Result<Option<T>, VaultError> {
            let store = self.store()?;
            let result = self.entry(&*store, account, label).and_then(|e| operation(&e));
            match classify(result) {
                Classified::Value(value) => Ok(Some(value)),
                Classified::Absent => Ok(None),
                Classified::Failed => {
                    let mut slot = self.store.lock().unwrap_or_else(|e| e.into_inner());
                    if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &store)) {
                        *slot = None;
                    }
                    Err(VaultError::Unavailable)
                }
            }
        }

        fn entry(
            &self,
            store: &CredentialStore,
            account: &str,
            label: Option<&str>,
        ) -> keyring_core::Result<Entry> {
            match label.filter(|_| self.backend.labels) {
                Some(label) => {
                    store.build(SERVICE, account, Some(&HashMap::from([("label", label)])))
                }
                None => store.build(SERVICE, account, None),
            }
        }
    }

    impl CredentialVault for OsVault {
        fn get(&self, account: &str) -> Result<Option<String>, VaultError> {
            self.with_entry(account, None, Entry::get_password)
        }

        fn set(&self, account: &str, label: &str, secret: &str) -> Result<(), VaultError> {
            self.with_entry(account, Some(label), |entry| entry.set_password(secret))?;
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), VaultError> {
            self.with_entry(account, None, Entry::delete_credential)?;
            Ok(())
        }
    }

    pub(super) enum Classified<T> {
        Value(T),
        Absent,
        Failed,
    }

    /// `NoEntry` is "not stored"; every other error, including variants added
    /// later, is a failure. The platform error is dropped without being read.
    pub(super) fn classify<T>(result: keyring_core::Result<T>) -> Classified<T> {
        match result {
            Ok(value) => Classified::Value(value),
            Err(Error::NoEntry) => Classified::Absent,
            Err(_) => Classified::Failed,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use keyring_core::mock;

        fn mock_vault() -> OsVault {
            OsVault::new(Backend {
                open: || Ok(mock::Store::new()?),
                labels: false,
            })
        }

        #[test]
        fn absent_items_are_not_failures() {
            let vault = mock_vault();
            assert_eq!(vault.get("jira https://a.example"), Ok(None));
            assert_eq!(vault.delete("jira https://a.example"), Ok(()));
        }

        #[test]
        fn set_get_overwrite_delete_roundtrip() {
            let vault = mock_vault();
            let account = "jira https://a.example";
            vault.set(account, "label", "one").unwrap();
            assert_eq!(vault.get(account).unwrap().as_deref(), Some("one"));
            vault.set(account, "label", "two").unwrap();
            assert_eq!(vault.get(account).unwrap().as_deref(), Some("two"));
            vault.delete(account).unwrap();
            assert_eq!(vault.get(account), Ok(None));
        }

        #[test]
        fn platform_errors_are_unavailable_and_the_store_is_reopened() {
            let vault = OsVault::new(Backend {
                open: || Ok(mock::Store::new()?),
                labels: false,
            });
            // A backend that cannot open reports Unavailable and is retried next call.
            let closed = OsVault::new(Backend {
                open: || Err(Error::NoDefaultStore),
                labels: false,
            });
            assert_eq!(closed.get("a"), Err(VaultError::Unavailable));
            assert_eq!(closed.set("a", "l", "s"), Err(VaultError::Unavailable));
            assert_eq!(closed.delete("a"), Err(VaultError::Unavailable));

            let store = vault.store().unwrap();
            let entry = store.build(SERVICE, "a", None).unwrap();
            entry.set_password("secret").unwrap();
            let credential = entry
                .as_any()
                .downcast_ref::<mock::Cred>()
                .expect("mock credential");
            credential.set_error(Error::NoStorageAccess("locked".into()));
            assert_eq!(vault.get("a"), Err(VaultError::Unavailable));
            // The failed call dropped the store; the next call opens a fresh one.
            assert!(vault.store.lock().unwrap().is_none());
            assert_eq!(vault.get("a"), Ok(None));
        }

        #[test]
        fn every_error_other_than_no_entry_fails() {
            assert!(matches!(
                classify::<()>(Err(Error::NoEntry)),
                Classified::Absent
            ));
            for error in [
                Error::BadEncoding(vec![0xff]),
                Error::Invalid("a".into(), "b".into()),
                Error::NotSupportedByStore("x".into()),
                Error::NoDefaultStore,
                Error::TooLong("a".into(), 1),
                Error::Ambiguous(Vec::new()),
                Error::PlatformFailure("boom".into()),
                Error::NoStorageAccess("locked".into()),
            ] {
                assert!(matches!(classify::<()>(Err(error)), Classified::Failed));
            }
        }
    }
}
