use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use serde::Serialize;
use uuid::Uuid;

use super::{AppError, AppResult};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCredentialSummary {
    pub credential_id: String,
    pub label: String,
    pub order: usize,
    pub is_active: bool,
}

#[derive(Default)]
pub struct SessionCredentialStore {
    values: Mutex<HashMap<String, Vec<NamedSessionCredential>>>,
}

struct NamedSessionCredential {
    credential_id: String,
    label: String,
    secret: SessionCredentialSecret,
}

struct SessionCredentialSecret(Vec<u8>);

impl SessionCredentialSecret {
    fn new(secret: String) -> AppResult<Self> {
        if secret.is_empty() {
            return Err(AppError::validation(
                "invalid_session_credential",
                "Session credential cannot be empty; remove it explicitly instead",
            ));
        }
        Ok(Self(secret.into_bytes()))
    }

    fn snapshot(&self) -> SessionCredentialValue {
        SessionCredentialValue(self.0.to_vec())
    }

    fn replace(&mut self, replacement: Self) {
        let mut previous = std::mem::replace(&mut self.0, replacement.into_bytes());
        previous.fill(0);
    }

    fn into_bytes(mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }

    fn clear(&mut self) {
        self.0.fill(0);
    }
}

impl Drop for SessionCredentialSecret {
    fn drop(&mut self) {
        self.clear();
    }
}

impl SessionCredentialStore {
    pub fn set(&self, provider_profile_id: String, credential: String) -> AppResult<()> {
        if credential.is_empty() {
            self.remove(&provider_profile_id)?;
            return Ok(());
        }

        let replacement = NamedSessionCredential {
            credential_id: Uuid::new_v4().to_string(),
            label: "Default".into(),
            secret: SessionCredentialSecret::new(credential)?,
        };
        let mut values = self.lock()?;
        if let Some(mut previous) = values.insert(provider_profile_id, vec![replacement]) {
            clear_credentials(&mut previous);
        }
        Ok(())
    }

    pub fn upsert(
        &self,
        provider_profile_id: String,
        credential_id: Option<String>,
        label: String,
        secret: String,
    ) -> AppResult<SessionCredentialSummary> {
        let label = validate_session_credential_label(label)?;
        let secret = SessionCredentialSecret::new(secret)?;
        let mut values = self.lock()?;
        let credentials = match credential_id.as_deref() {
            Some(credential_id) => values
                .get_mut(&provider_profile_id)
                .ok_or_else(|| credential_not_found(credential_id))?,
            None => values.entry(provider_profile_id).or_default(),
        };

        if credentials.iter().any(|credential| {
            credential.label == label
                && credential_id.as_deref() != Some(credential.credential_id.as_str())
        }) {
            return Err(AppError::validation(
                "duplicate_credential_label",
                "Credential labels must be unique within a Provider Profile",
            ));
        }

        let index = match credential_id {
            Some(credential_id) => {
                let index = credentials
                    .iter()
                    .position(|credential| credential.credential_id == credential_id)
                    .ok_or_else(|| credential_not_found(&credential_id))?;
                let credential = &mut credentials[index];
                credential.label = label;
                credential.secret.replace(secret);
                index
            }
            None => {
                credentials.push(NamedSessionCredential {
                    credential_id: Uuid::new_v4().to_string(),
                    label,
                    secret,
                });
                credentials.len() - 1
            }
        };

        Ok(summary_for(&credentials[index], index))
    }

    pub fn summaries(&self, provider_profile_id: &str) -> AppResult<Vec<SessionCredentialSummary>> {
        let values = self.lock()?;
        Ok(values
            .get(provider_profile_id)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, credential)| summary_for(credential, index))
            .collect())
    }

    pub fn activate(&self, provider_profile_id: &str, credential_id: &str) -> AppResult<()> {
        let mut values = self.lock()?;
        let credentials = values
            .get_mut(provider_profile_id)
            .ok_or_else(|| credential_not_found(credential_id))?;
        let index = credentials
            .iter()
            .position(|credential| credential.credential_id == credential_id)
            .ok_or_else(|| credential_not_found(credential_id))?;
        if index != 0 {
            let credential = credentials.remove(index);
            credentials.insert(0, credential);
        }
        Ok(())
    }

    pub fn reorder(
        &self,
        provider_profile_id: &str,
        ordered_credential_ids: &[String],
    ) -> AppResult<()> {
        let mut values = self.lock()?;
        let credentials = values
            .get_mut(provider_profile_id)
            .ok_or_else(invalid_credential_order)?;

        let requested: HashSet<&str> = ordered_credential_ids.iter().map(String::as_str).collect();
        let exact_permutation = ordered_credential_ids.len() == credentials.len()
            && requested.len() == credentials.len()
            && credentials
                .iter()
                .all(|credential| requested.contains(credential.credential_id.as_str()));
        if !exact_permutation {
            return Err(invalid_credential_order());
        }
        if ordered_credential_ids.first().map(String::as_str)
            != credentials
                .first()
                .map(|credential| credential.credential_id.as_str())
        {
            return Err(AppError::validation(
                "active_credential_requires_activation",
                "Credential ordering cannot change the active credential; activate it explicitly",
            ));
        }

        let mut by_id: HashMap<String, NamedSessionCredential> = credentials
            .drain(..)
            .map(|credential| (credential.credential_id.clone(), credential))
            .collect();
        for credential_id in ordered_credential_ids {
            credentials.push(
                by_id
                    .remove(credential_id)
                    .expect("exact permutation was validated"),
            );
        }
        Ok(())
    }

    pub fn remove_one(&self, provider_profile_id: &str, credential_id: &str) -> AppResult<()> {
        let mut values = self.lock()?;
        let should_remove_profile = {
            let credentials = values
                .get_mut(provider_profile_id)
                .ok_or_else(|| credential_not_found(credential_id))?;
            let index = credentials
                .iter()
                .position(|credential| credential.credential_id == credential_id)
                .ok_or_else(|| credential_not_found(credential_id))?;
            if index == 0 && credentials.len() > 1 {
                return Err(AppError::validation(
                    "active_credential_requires_replacement",
                    "Activate another credential before removing the active credential",
                ));
            }
            let mut removed = credentials.remove(index);
            removed.secret.clear();
            credentials.is_empty()
        };
        if should_remove_profile {
            values.remove(provider_profile_id);
        }
        Ok(())
    }

    pub fn get(&self, provider_profile_id: &str) -> AppResult<Option<SessionCredentialValue>> {
        let credential = {
            let values = self.lock()?;
            values
                .get(provider_profile_id)
                .and_then(|credentials| credentials.first())
                .map(|credential| credential.secret.snapshot())
        };
        Ok(credential)
    }

    pub fn selected_lookup(
        &self,
        provider_profile_id: &str,
        credential_id: &str,
    ) -> AppResult<Arc<dyn SessionCredentialLookup>> {
        let secret = {
            let values = self.lock()?;
            let credentials = values
                .get(provider_profile_id)
                .ok_or_else(|| credential_not_found(credential_id))?;
            let credential = credentials
                .iter()
                .find(|credential| credential.credential_id == credential_id)
                .ok_or_else(|| credential_not_found(credential_id))?;
            SessionCredentialSecret(credential.secret.0.to_vec())
        };
        Ok(Arc::new(SelectedSessionCredentialLookup {
            provider_profile_id: provider_profile_id.to_owned(),
            secret,
        }))
    }

    pub fn contains(&self, provider_profile_id: &str) -> AppResult<bool> {
        Ok(self
            .lock()?
            .get(provider_profile_id)
            .is_some_and(|credentials| !credentials.is_empty()))
    }

    pub fn remove(&self, provider_profile_id: &str) -> AppResult<()> {
        if let Some(mut credentials) = self.lock()?.remove(provider_profile_id) {
            clear_credentials(&mut credentials);
        }
        Ok(())
    }

    pub fn clear(&self) -> AppResult<()> {
        let mut values = self.lock()?;
        for credentials in values.values_mut() {
            clear_credentials(credentials);
        }
        values.clear();
        Ok(())
    }

    fn lock(&self) -> AppResult<MutexGuard<'_, HashMap<String, Vec<NamedSessionCredential>>>> {
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
            for credentials in values.values_mut() {
                clear_credentials(credentials);
            }
            values.clear();
        }
    }
}

fn summary_for(credential: &NamedSessionCredential, index: usize) -> SessionCredentialSummary {
    SessionCredentialSummary {
        credential_id: credential.credential_id.clone(),
        label: credential.label.clone(),
        order: index,
        is_active: index == 0,
    }
}

pub fn validate_session_credential_label(label: String) -> AppResult<String> {
    let label = label.trim().to_owned();
    let length = label.chars().count();
    if !(1..=64).contains(&length)
        || label
            .chars()
            .any(|character| character.is_control() || is_bidi_control(character))
    {
        return Err(AppError::validation(
            "invalid_credential_label",
            "Credential label must contain 1 to 64 characters without control characters",
        ));
    }
    Ok(label)
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn credential_not_found(credential_id: &str) -> AppError {
    AppError::validation(
        "session_credential_not_found",
        format!("Session credential {credential_id} was not found"),
    )
}

fn invalid_credential_order() -> AppError {
    AppError::validation(
        "invalid_credential_order",
        "Credential order must be an exact permutation of the Provider Profile credentials",
    )
}

fn clear_credentials(credentials: &mut Vec<NamedSessionCredential>) {
    for credential in credentials.iter_mut() {
        credential.secret.clear();
    }
    credentials.clear();
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

struct SelectedSessionCredentialLookup {
    provider_profile_id: String,
    secret: SessionCredentialSecret,
}

impl SessionCredentialLookup for SelectedSessionCredentialLookup {
    fn credential_for(
        &self,
        provider_profile_id: &str,
    ) -> AppResult<Option<SessionCredentialValue>> {
        Ok((provider_profile_id == self.provider_profile_id).then(|| self.secret.snapshot()))
    }
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

    fn upsert(
        store: &SessionCredentialStore,
        profile_id: &str,
        credential_id: Option<&str>,
        label: &str,
        secret: &str,
    ) -> SessionCredentialSummary {
        store
            .upsert(
                profile_id.to_owned(),
                credential_id.map(str::to_owned),
                label.to_owned(),
                secret.to_owned(),
            )
            .expect("credential upsert succeeds")
    }

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

    #[test]
    fn named_credentials_keep_stable_order_and_first_is_active() {
        let store = SessionCredentialStore::default();
        let first = upsert(&store, "profile-1", None, "Primary", "sk-primary");
        let second = upsert(&store, "profile-1", None, "Backup", "sk-backup");

        assert_ne!(first.credential_id, second.credential_id);
        assert_eq!(
            store
                .summaries("profile-1")
                .expect("summaries are readable"),
            vec![
                SessionCredentialSummary {
                    credential_id: first.credential_id,
                    label: "Primary".into(),
                    order: 0,
                    is_active: true,
                },
                SessionCredentialSummary {
                    credential_id: second.credential_id,
                    label: "Backup".into(),
                    order: 1,
                    is_active: false,
                },
            ]
        );
        assert_eq!(
            store
                .get("profile-1")
                .expect("store is readable")
                .expect("active credential exists")
                .as_str()
                .expect("credential is UTF-8"),
            "sk-primary"
        );
    }

    #[test]
    fn upsert_replaces_metadata_and_secret_without_changing_order() {
        let store = SessionCredentialStore::default();
        let primary = upsert(&store, "profile-1", None, "Primary", "old-secret");
        let backup = upsert(&store, "profile-1", None, "Backup", "backup-secret");

        let replaced = upsert(
            &store,
            "profile-1",
            Some(&primary.credential_id),
            "Primary renamed",
            "new-secret",
        );

        assert_eq!(replaced.credential_id, primary.credential_id);
        assert_eq!(replaced.order, 0);
        assert!(replaced.is_active);
        assert_eq!(
            store
                .summaries("profile-1")
                .expect("summaries are readable")
                .into_iter()
                .map(|summary| summary.credential_id)
                .collect::<Vec<_>>(),
            vec![primary.credential_id, backup.credential_id]
        );
        assert_eq!(
            store
                .get("profile-1")
                .expect("store is readable")
                .expect("active credential exists")
                .as_str()
                .expect("credential is UTF-8"),
            "new-secret"
        );
    }

    #[test]
    fn activate_moves_one_credential_to_front_stably() {
        let store = SessionCredentialStore::default();
        let first = upsert(&store, "profile-1", None, "One", "secret-1");
        let second = upsert(&store, "profile-1", None, "Two", "secret-2");
        let third = upsert(&store, "profile-1", None, "Three", "secret-3");

        store
            .activate("profile-1", &second.credential_id)
            .expect("credential activates");

        assert_eq!(
            store
                .summaries("profile-1")
                .expect("summaries are readable")
                .into_iter()
                .map(|summary| summary.credential_id)
                .collect::<Vec<_>>(),
            vec![
                second.credential_id,
                first.credential_id,
                third.credential_id
            ]
        );
        assert_eq!(
            store
                .get("profile-1")
                .expect("store is readable")
                .expect("active credential exists")
                .as_str()
                .expect("credential is UTF-8"),
            "secret-2"
        );
    }

    #[test]
    fn selected_lookup_snapshots_an_exact_non_active_credential() {
        let store = SessionCredentialStore::default();
        let primary = upsert(&store, "profile-1", None, "Primary", "secret-primary");
        let backup = upsert(&store, "profile-1", None, "Backup", "secret-backup");
        let lookup = store
            .selected_lookup("profile-1", &backup.credential_id)
            .expect("selected lookup is created");

        assert_eq!(
            lookup
                .credential_for("profile-1")
                .expect("lookup succeeds")
                .expect("selected credential exists")
                .as_str()
                .expect("credential is valid UTF-8"),
            "secret-backup"
        );
        assert!(
            lookup
                .credential_for("different-profile")
                .expect("lookup succeeds")
                .is_none()
        );
        assert_eq!(
            store
                .summaries("profile-1")
                .expect("summaries remain readable")[0]
                .credential_id,
            primary.credential_id,
            "creating an exact lookup must not mutate the active credential"
        );
    }

    #[test]
    fn reorder_requires_an_exact_permutation() {
        let store = SessionCredentialStore::default();
        let first = upsert(&store, "profile-1", None, "One", "secret-1");
        let second = upsert(&store, "profile-1", None, "Two", "secret-2");
        let third = upsert(&store, "profile-1", None, "Three", "secret-3");

        store
            .reorder(
                "profile-1",
                &[
                    first.credential_id.clone(),
                    third.credential_id.clone(),
                    second.credential_id.clone(),
                ],
            )
            .expect("exact permutation is accepted");
        assert_eq!(
            store
                .summaries("profile-1")
                .expect("summaries are readable")
                .into_iter()
                .map(|summary| summary.credential_id)
                .collect::<Vec<_>>(),
            vec![
                first.credential_id.clone(),
                third.credential_id.clone(),
                second.credential_id.clone()
            ]
        );

        let activation_error = store
            .reorder(
                "profile-1",
                &[
                    third.credential_id.clone(),
                    first.credential_id.clone(),
                    second.credential_id.clone(),
                ],
            )
            .expect_err("ordering cannot implicitly switch the active credential");
        assert_eq!(
            activation_error.code,
            "active_credential_requires_activation"
        );

        for invalid in [
            vec![first.credential_id.clone(), second.credential_id.clone()],
            vec![
                first.credential_id.clone(),
                first.credential_id.clone(),
                third.credential_id.clone(),
            ],
            vec![
                first.credential_id.clone(),
                second.credential_id.clone(),
                "unknown".into(),
            ],
        ] {
            let error = store
                .reorder("profile-1", &invalid)
                .expect_err("non-permutation is rejected");
            assert_eq!(error.code, "invalid_credential_order");
        }
    }

    #[test]
    fn removing_active_with_backups_requires_explicit_replacement() {
        let store = SessionCredentialStore::default();
        let first = upsert(&store, "profile-1", None, "One", "secret-1");
        let second = upsert(&store, "profile-1", None, "Two", "secret-2");

        let error = store
            .remove_one("profile-1", &first.credential_id)
            .expect_err("active credential cannot be removed while backups exist");
        assert_eq!(error.code, "active_credential_requires_replacement");
        let summaries = store.summaries("profile-1").expect("store is readable");
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].credential_id, first.credential_id);
        assert!(summaries[0].is_active);

        store
            .activate("profile-1", &second.credential_id)
            .expect("replacement is explicitly activated");
        store
            .remove_one("profile-1", &first.credential_id)
            .expect("the old credential is now a removable backup");
        let summaries = store.summaries("profile-1").expect("store is readable");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].credential_id, second.credential_id);
        assert!(summaries[0].is_active);

        store
            .remove_one("profile-1", &second.credential_id)
            .expect("last credential is removed");
        assert!(!store.contains("profile-1").expect("store is readable"));
        assert!(
            store
                .summaries("profile-1")
                .expect("store is readable")
                .is_empty()
        );
    }

    #[test]
    fn named_credential_validation_is_explicit_and_non_destructive() {
        let store = SessionCredentialStore::default();
        let primary = upsert(&store, "profile-1", None, "Primary", "original-secret");

        for invalid_label in ["", "   ", "bad\u{0007}label", "bad\u{202e}label"] {
            let error = store
                .upsert(
                    "profile-1".into(),
                    None,
                    invalid_label.into(),
                    "secret".into(),
                )
                .expect_err("invalid label is rejected");
            assert_eq!(error.code, "invalid_credential_label");
        }
        let too_long = "x".repeat(65);
        let error = store
            .upsert("profile-1".into(), None, too_long, "secret".into())
            .expect_err("overlong label is rejected");
        assert_eq!(error.code, "invalid_credential_label");

        let error = store
            .upsert(
                "profile-1".into(),
                None,
                "Primary".into(),
                "duplicate".into(),
            )
            .expect_err("duplicate label is rejected");
        assert_eq!(error.code, "duplicate_credential_label");

        let error = store
            .upsert(
                "profile-1".into(),
                Some(primary.credential_id.clone()),
                "Primary".into(),
                String::new(),
            )
            .expect_err("empty replacement is rejected");
        assert_eq!(error.code, "invalid_session_credential");
        assert_eq!(
            store
                .get("profile-1")
                .expect("store is readable")
                .expect("credential remains")
                .as_str()
                .expect("credential is UTF-8"),
            "original-secret"
        );

        let error = store
            .upsert(
                "profile-1".into(),
                Some("unknown".into()),
                "Unknown".into(),
                "secret".into(),
            )
            .expect_err("unknown id is rejected");
        assert_eq!(error.code, "session_credential_not_found");
    }

    #[test]
    fn summaries_and_debug_output_never_expose_secrets() {
        let store = SessionCredentialStore::default();
        upsert(
            &store,
            "profile-1",
            None,
            "Production key",
            "super-secret-value",
        );

        let summaries = store.summaries("profile-1").expect("store is readable");
        let json = serde_json::to_string(&summaries).expect("summaries serialize");
        assert!(json.contains("Production key"));
        assert!(!json.contains("super-secret-value"));
        assert!(!json.contains("secret"));
        assert_eq!(
            format!(
                "{:?}",
                store
                    .get("profile-1")
                    .expect("store is readable")
                    .expect("credential exists")
            ),
            "SessionCredentialValue(<redacted>)"
        );
    }

    #[test]
    fn profiles_are_isolated_and_clear_removes_everything() {
        let store = SessionCredentialStore::default();
        upsert(&store, "profile-1", None, "One", "secret-1");
        upsert(&store, "profile-2", None, "Two", "secret-2");

        assert_eq!(
            store
                .summaries("profile-1")
                .expect("store is readable")
                .len(),
            1
        );
        assert_eq!(
            store
                .summaries("profile-2")
                .expect("store is readable")
                .len(),
            1
        );

        store.clear().expect("store clears");
        assert!(!store.contains("profile-1").expect("store is readable"));
        assert!(!store.contains("profile-2").expect("store is readable"));
    }
}
