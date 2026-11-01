//! Retry migrated credential deletions without delaying application startup.

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::{AppHandle, Manager};

use crate::{app_state::AppState, database::legacy_provider_cleanup as pending};

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if cleanup(&state.db, &state.credential_service).is_err() {
            tracing::warn!("Could not update legacy credential cleanup; retrying next launch");
        }
    });
}

fn cleanup(db: &Mutex<Connection>, service: &str) -> rusqlite::Result<()> {
    let credentials = pending::pending_credentials(&db.lock().unwrap())?;
    for (key, username) in credentials {
        // Do not hold SQLite's mutex during OS vault access. Only the current
        // bundle's service is addressed; QA cannot delete production credentials.
        let deleted =
            keyring::Entry::new(service, &username).and_then(|entry| entry.delete_credential());
        match deleted {
            Ok(()) | Err(keyring::Error::NoEntry) => {
                pending::credential_deleted(&db.lock().unwrap(), &key)?;
            }
            Err(_) => {
                tracing::warn!("Could not delete a legacy credential; retrying next launch");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    const QA_SERVICE: &str = "com.trainerpro.desktop.qa.provider-credentials";
    const REGULAR_SERVICE: &str = "com.trainerpro.desktop.provider-credentials";

    struct MockVault {
        locked: Arc<AtomicBool>,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl CredentialBuilderApi for MockVault {
        fn build(
            &self,
            _: Option<&str>,
            service: &str,
            user: &str,
        ) -> keyring::Result<Box<Credential>> {
            self.calls
                .lock()
                .unwrap()
                .push((service.into(), user.into()));
            let credential = keyring::mock::MockCredential::default();
            if user != "intervals_icu:missing" {
                credential.set_password("synthetic-test-key")?;
            }
            if user == "intervals_icu:locked" && self.locked.load(Ordering::SeqCst) {
                credential.set_error(keyring::Error::Invalid("store".into(), "locked".into()));
            }
            Ok(Box::new(credential))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn vault_cleanup_retries_failures_accepts_missing_entries_and_scopes_services() {
        let locked = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(Mutex::new(Vec::new()));
        keyring::set_default_credential_builder(Box::new(MockVault {
            locked: locked.clone(),
            calls: calls.clone(),
        }));
        let db = Mutex::new(crate::database::test_connection());
        for account in ["present", "missing", "locked"] {
            crate::database::settings::set(
                &db.lock().unwrap(),
                &format!("legacy_intervals_icu_credential:{account}"),
                &format!("intervals_icu:{account}"),
            )
            .unwrap();
        }
        cleanup(&db, QA_SERVICE).unwrap();
        let remaining = pending::pending_credentials(&db.lock().unwrap()).unwrap();
        assert_eq!(
            remaining,
            [(
                "legacy_intervals_icu_credential:locked".into(),
                "intervals_icu:locked".into()
            )]
        );
        assert_eq!(calls.lock().unwrap().len(), 3);
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .all(|(service, _)| service == QA_SERVICE));

        locked.store(false, Ordering::SeqCst);
        cleanup(&db, QA_SERVICE).unwrap();
        assert!(pending::pending_credentials(&db.lock().unwrap())
            .unwrap()
            .is_empty());
        assert_eq!(calls.lock().unwrap().len(), 4);
        cleanup(&db, QA_SERVICE).unwrap();
        assert_eq!(calls.lock().unwrap().len(), 4);

        // A regular-identity run uses its own service even for the same username.
        crate::database::settings::set(
            &db.lock().unwrap(),
            "legacy_intervals_icu_credential:present",
            "intervals_icu:present",
        )
        .unwrap();
        cleanup(&db, REGULAR_SERVICE).unwrap();
        assert_eq!(
            calls.lock().unwrap().last(),
            Some(&(REGULAR_SERVICE.into(), "intervals_icu:present".into()))
        );
        keyring::set_default_credential_builder(keyring::default::default_credential_builder());
    }
}
