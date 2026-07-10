//! Pi-like Credential persistence at `{data_dir}/auth.json`.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const AUTH_FILE_MODE: u32 = 0o600;
const DATA_DIR_MODE: u32 = 0o700;

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("failed to access Credential store: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse auth.json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("failed to acquire auth.json lock")]
    LockTimeout,
}

/// Stored Credential for one Provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    OAuth {
        access: String,
        refresh: String,
        /// Expiry as unix epoch milliseconds (pi-compatible).
        expires: u64,
    },
    ApiKey {
        key: String,
    },
}

type AuthMap = BTreeMap<String, Credential>;

/// File-backed Credential store (`auth.json`).
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    /// Open (or create parent for) `{data_dir}/auth.json`.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, CredentialError> {
        let data_dir = data_dir.as_ref();
        fs::create_dir_all(data_dir)?;
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(data_dir)?.permissions();
            perms.set_mode(DATA_DIR_MODE);
            fs::set_permissions(data_dir, perms)?;
        }
        Ok(Self {
            path: data_dir.join("auth.json"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read a Credential by provider id. Missing entry → `None`.
    pub fn read(&self, provider_id: &str) -> Result<Option<Credential>, CredentialError> {
        let map = self.load_unlocked()?;
        Ok(map.get(provider_id).cloned())
    }

    /// Set a Credential (login / replace).
    pub fn set(&self, provider_id: &str, credential: Credential) -> Result<(), CredentialError> {
        self.modify(provider_id, |_| Ok(Some(credential)))?;
        Ok(())
    }

    /// Delete a Credential (logout).
    pub fn delete(&self, provider_id: &str) -> Result<(), CredentialError> {
        self.modify(provider_id, |_| Ok(None))?;
        Ok(())
    }

    /// Serialized read-modify-write. `fn` sees the current entry; return
    /// `Some(cred)` to write, `None` to remove the provider key.
    ///
    /// This is the only write path — refresh must run inside `modify` so
    /// concurrent refresh cannot race.
    pub fn modify<F>(&self, provider_id: &str, f: F) -> Result<Option<Credential>, CredentialError>
    where
        F: FnOnce(Option<Credential>) -> Result<Option<Credential>, CredentialError>,
    {
        self.with_lock(|map| {
            let current = map.get(provider_id).cloned();
            let next = f(current)?;
            match &next {
                Some(cred) => {
                    map.insert(provider_id.to_string(), cred.clone());
                }
                None => {
                    map.remove(provider_id);
                }
            }
            Ok(next)
        })
    }

    fn with_lock<T, F>(&self, f: F) -> Result<T, CredentialError>
    where
        F: FnOnce(&mut AuthMap) -> Result<T, CredentialError>,
    {
        self.ensure_file()?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .mode(AUTH_FILE_MODE)
            .open(&self.path)?;

        acquire_exclusive_lock(&file)?;

        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let mut map: AuthMap = if contents.trim().is_empty() {
            AuthMap::new()
        } else {
            serde_json::from_str(&contents)?
        };

        let result = f(&mut map)?;

        let serialized = serde_json::to_string_pretty(&map)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(serialized.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        chmod_0600(&self.path)?;

        // Lock released when `file` drops (and we unlock explicitly).
        let _ = file.unlock();
        Ok(result)
    }

    fn load_unlocked(&self) -> Result<AuthMap, CredentialError> {
        if !self.path.exists() {
            return Ok(AuthMap::new());
        }
        let text = fs::read_to_string(&self.path)?;
        if text.trim().is_empty() {
            return Ok(AuthMap::new());
        }
        Ok(serde_json::from_str(&text)?)
    }

    fn ensure_file(&self) -> Result<(), CredentialError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !self.path.exists() {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(AUTH_FILE_MODE)
                .open(&self.path)?;
            file.write_all(b"{}\n")?;
            file.sync_all()?;
            chmod_0600(&self.path)?;
        }
        Ok(())
    }
}

fn chmod_0600(path: &Path) -> Result<(), CredentialError> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(AUTH_FILE_MODE);
    fs::set_permissions(path, perms)?;
    Ok(())
}

fn acquire_exclusive_lock(file: &File) -> Result<(), CredentialError> {
    // Poll until the lock is free. Short sleeps keep contention tests and
    // concurrent processes from timing out on slow CI runners.
    const MAX_WAIT: Duration = Duration::from_secs(5);
    let deadline = Instant::now() + MAX_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(CredentialError::LockTimeout);
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(TryLockError::Error(err)) => return Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::TempDir;

    #[test]
    fn round_trip_api_key_and_oauth() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::open(tmp.path()).unwrap();

        store
            .set(
                "openai",
                Credential::ApiKey {
                    key: "sk-test".into(),
                },
            )
            .unwrap();
        store
            .set(
                "codex",
                Credential::OAuth {
                    access: "a".into(),
                    refresh: "r".into(),
                    expires: 1_700_000_000_000,
                },
            )
            .unwrap();

        assert_eq!(
            store.read("openai").unwrap(),
            Some(Credential::ApiKey {
                key: "sk-test".into()
            })
        );
        assert_eq!(
            store.read("codex").unwrap(),
            Some(Credential::OAuth {
                access: "a".into(),
                refresh: "r".into(),
                expires: 1_700_000_000_000,
            })
        );
        assert_eq!(store.read("missing").unwrap(), None);
    }

    #[test]
    fn delete_removes_provider_entry() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::open(tmp.path()).unwrap();
        store
            .set("codex", Credential::ApiKey { key: "k".into() })
            .unwrap();
        store.delete("codex").unwrap();
        assert_eq!(store.read("codex").unwrap(), None);
    }

    #[test]
    fn auth_json_created_with_mode_0600() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::open(tmp.path()).unwrap();
        store
            .set("codex", Credential::ApiKey { key: "k".into() })
            .unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn modify_serializes_concurrent_updates() {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(CredentialStore::open(tmp.path()).unwrap());
        store
            .set("codex", Credential::ApiKey { key: "0".into() })
            .unwrap();

        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                store
                    .modify("codex", |current| {
                        let n: u32 = match current {
                            Some(Credential::ApiKey { key }) => key.parse().unwrap_or(0),
                            _ => 0,
                        };
                        Ok(Some(Credential::ApiKey {
                            key: (n + 1).to_string(),
                        }))
                    })
                    .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        match store.read("codex").unwrap() {
            Some(Credential::ApiKey { key }) => assert_eq!(key, "8"),
            other => panic!("unexpected credential: {other:?}"),
        }
    }
}
