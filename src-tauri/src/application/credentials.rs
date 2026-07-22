use std::{
    collections::HashMap,
    fmt,
    sync::{Mutex, MutexGuard},
};

use super::{AppError, AppResult};

#[derive(Default)]
pub struct SessionCredentialStore {
    values: Mutex<HashMap<String, Vec<u8>>>,
}

impl SessionCredentialStore {
    pub fn set(&self, provider_profile_id: String, credential: String) -> AppResult<()> {
        let mut values = self.lock()?;
        let mut bytes = credential.into_bytes();
        if bytes.is_empty() {
            if let Some(mut previous) = values.remove(&provider_profile_id) {
                previous.fill(0);
            }
            return Ok(());
        }

        if let Some(mut previous) = values.insert(provider_profile_id, std::mem::take(&mut bytes)) {
            previous.fill(0);
        }
        bytes.fill(0);
        Ok(())
    }

    pub fn get(&self, provider_profile_id: &str) -> AppResult<Option<SessionCredentialValue>> {
        let values = self.lock()?;
        Ok(values
            .get(provider_profile_id)
            .cloned()
            .map(SessionCredentialValue))
    }

    pub fn contains(&self, provider_profile_id: &str) -> AppResult<bool> {
        Ok(self.lock()?.contains_key(provider_profile_id))
    }

    pub fn remove(&self, provider_profile_id: &str) -> AppResult<()> {
        if let Some(mut value) = self.lock()?.remove(provider_profile_id) {
            value.fill(0);
        }
        Ok(())
    }

    pub fn clear(&self) -> AppResult<()> {
        let mut values = self.lock()?;
        for value in values.values_mut() {
            value.fill(0);
        }
        values.clear();
        Ok(())
    }

    fn lock(&self) -> AppResult<MutexGuard<'_, HashMap<String, Vec<u8>>>> {
        self.values.lock().map_err(|_| {
            AppError::internal(
                "credential_store_unavailable",
                "Session credential store is unavailable",
            )
        })
    }
}

impl Drop for SessionCredentialStore {
    fn drop(&mut self) {
        if let Ok(values) = self.values.get_mut() {
            for value in values.values_mut() {
                value.fill(0);
            }
            values.clear();
        }
    }
}

pub struct SessionCredentialValue(Vec<u8>);

impl SessionCredentialValue {
    pub fn as_str(&self) -> AppResult<&str> {
        std::str::from_utf8(&self.0).map_err(|_| {
            AppError::validation(
                "invalid_session_credential",
                "Session credential must be valid UTF-8",
            )
        })
    }
}

impl fmt::Debug for SessionCredentialValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionCredentialValue(<redacted>)")
    }
}

impl Drop for SessionCredentialValue {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

pub trait SessionCredentialLookup: Send + Sync {
    fn credential_for(
        &self,
        provider_profile_id: &str,
    ) -> AppResult<Option<SessionCredentialValue>>;
}

impl SessionCredentialLookup for SessionCredentialStore {
    fn credential_for(
        &self,
        provider_profile_id: &str,
    ) -> AppResult<Option<SessionCredentialValue>> {
        self.get(provider_profile_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_session_scoped_and_never_debugged() {
        let store = SessionCredentialStore::default();
        store
            .set("profile-1".into(), "sk-not-for-storage".into())
            .expect("credential is accepted");

        let credential = store
            .get("profile-1")
            .expect("credential store is readable")
            .expect("credential is present");
        assert_eq!(
            credential.as_str().expect("credential is UTF-8"),
            "sk-not-for-storage"
        );
        assert_eq!(
            format!("{credential:?}"),
            "SessionCredentialValue(<redacted>)"
        );

        store
            .set("profile-1".into(), String::new())
            .expect("empty credential clears the profile");
        assert!(!store.contains("profile-1").expect("store is readable"));
    }
}
