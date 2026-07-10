//! Exa Web Backend: API-key Credential Login.

use crate::credentials::{Credential, CredentialError, CredentialStore};

/// Credential / config id for the Exa Web Backend.
pub const WEB_BACKEND_ID: &str = "exa";

/// Persist an Exa API-key Credential, overwriting any existing entry.
pub fn login_api_key(store: &CredentialStore, key: &str) -> Result<(), CredentialError> {
    store.set(
        WEB_BACKEND_ID,
        Credential::ApiKey {
            key: key.to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn login_api_key_persists_and_overwrites() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::open(tmp.path()).unwrap();

        login_api_key(&store, "key-one").expect("first login");
        assert_eq!(
            store.read(WEB_BACKEND_ID).unwrap(),
            Some(Credential::ApiKey {
                key: "key-one".into()
            })
        );

        login_api_key(&store, "key-two").expect("re-login");
        assert_eq!(
            store.read(WEB_BACKEND_ID).unwrap(),
            Some(Credential::ApiKey {
                key: "key-two".into()
            })
        );
    }
}
