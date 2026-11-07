//! Origin-bound credentials. Values and origins are stored together in the OS vault.
use crate::sources::KeyStore;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use url::{Host, Url};

pub(super) fn validate_id(id: &str) -> Result<(), String> {
    if !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        Ok(())
    } else {
        Err("Invalid Integration or field id".into())
    }
}

pub fn origin(base: &str) -> Result<String, String> {
    let url = Url::parse(base.trim()).map_err(|_| "Invalid Integration URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTP or HTTPS URL without credentials, query or fragment".into());
    }
    Ok(url.origin().ascii_serialization())
}

/// Check the typed host, never a DNS result. Redirects are disabled by HttpFetch.
pub fn credential_transport(base: &str) -> Result<(), String> {
    origin(base)?;
    let u = Url::parse(base.trim()).unwrap();
    let private = match u.host() {
        Some(Host::Domain(h)) => {
            h.eq_ignore_ascii_case("localhost") || h.eq_ignore_ascii_case("localhost.")
        }
        Some(Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private(),
        Some(Host::Ipv6(ip)) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|v| v.is_loopback() || v.is_private())
        }
        None => false,
    };
    if u.scheme() == "https" || private {
        Ok(())
    } else {
        Err("Credentials require HTTPS or an explicit loopback or private address".into())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Secret {
    pub origin: String,
    pub value: String,
}
pub trait SecretStore: Send + Sync {
    fn get(&self, integration: &str, field: &str) -> Result<Option<Secret>, String>;
    fn put(&self, integration: &str, field: &str, secret: Secret) -> Result<(), String>;
    fn delete(&self, integration: &str, field: &str) -> Result<(), String>;
    fn legacy(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn delete_legacy(&self) -> Result<(), String> {
        Ok(())
    }
}
#[derive(Default)]
pub struct MemorySecrets(Mutex<BTreeMap<(String, String), Secret>>);
impl SecretStore for MemorySecrets {
    fn get(&self, i: &str, f: &str) -> Result<Option<Secret>, String> {
        validate_id(i)?;
        validate_id(f)?;
        Ok(self.0.lock().unwrap().get(&(i.into(), f.into())).cloned())
    }
    fn put(&self, i: &str, f: &str, s: Secret) -> Result<(), String> {
        validate_id(i)?;
        validate_id(f)?;
        self.0.lock().unwrap().insert((i.into(), f.into()), s);
        Ok(())
    }
    fn delete(&self, i: &str, f: &str) -> Result<(), String> {
        validate_id(i)?;
        validate_id(f)?;
        self.0.lock().unwrap().remove(&(i.into(), f.into()));
        Ok(())
    }
}
pub struct OsSecrets;
fn entry(i: &str, f: &str) -> Result<keyring::Entry, String> {
    validate_id(i)?;
    validate_id(f)?;
    keyring::Entry::new("touchdeck", &format!("integration:{i}:{f}"))
        .map_err(|_| "Credential store unavailable".into())
}
fn read(e: keyring::Entry) -> Result<Option<String>, String> {
    match e.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Credential store unavailable".into()),
    }
}
fn delete(e: keyring::Entry) -> Result<(), String> {
    match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Credential store unavailable".into()),
    }
}
fn legacy_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new("touchdeck", "cliproxyapi-management-key")
        .map_err(|_| "Credential store unavailable".into())
}
impl SecretStore for OsSecrets {
    fn get(&self, i: &str, f: &str) -> Result<Option<Secret>, String> {
        read(entry(i, f)?)?
            .map(|s| serde_json::from_str(&s).map_err(|_| "Invalid stored credential".into()))
            .transpose()
    }
    fn put(&self, i: &str, f: &str, s: Secret) -> Result<(), String> {
        entry(i, f)?
            .set_password(&serde_json::to_string(&s).unwrap())
            .map_err(|_| "Credential store unavailable".into())
    }
    fn delete(&self, i: &str, f: &str) -> Result<(), String> {
        delete(entry(i, f)?)
    }
    fn legacy(&self) -> Result<Option<String>, String> {
        read(legacy_entry()?)
    }
    fn delete_legacy(&self) -> Result<(), String> {
        delete(legacy_entry()?)
    }
}

/// Credentials that belong to the app, not to an Integration, such as the Brandfetch API key.
/// Same vault and value shape as [`OsSecrets`], under `app:{owner}:{field}` entries.
pub struct OsAppSecrets;
fn app_entry(owner: &str, field: &str) -> Result<keyring::Entry, String> {
    validate_id(owner)?;
    validate_id(field)?;
    keyring::Entry::new("touchdeck", &format!("app:{owner}:{field}"))
        .map_err(|_| "Credential store unavailable".into())
}
impl SecretStore for OsAppSecrets {
    fn get(&self, owner: &str, field: &str) -> Result<Option<Secret>, String> {
        read(app_entry(owner, field)?)?
            .map(|s| serde_json::from_str(&s).map_err(|_| "Invalid stored credential".into()))
            .transpose()
    }
    fn put(&self, owner: &str, field: &str, s: Secret) -> Result<(), String> {
        app_entry(owner, field)?
            .set_password(&serde_json::to_string(&s).unwrap())
            .map_err(|_| "Credential store unavailable".into())
    }
    fn delete(&self, owner: &str, field: &str) -> Result<(), String> {
        delete(app_entry(owner, field)?)
    }
}

/// Shared with the legacy Tauri adapter. Pollers ask for their immutable URL, so a
/// reconfiguration cannot give an old poller a new origin's credential.
pub struct BoundKeys {
    store: Arc<dyn SecretStore>,
    origin: Mutex<String>,
}
impl BoundKeys {
    /// A previously saved invalid URL must remain repairable. An empty binding
    /// fails closed until a valid config is saved and credentials are re-entered.
    pub fn from_config(store: Arc<dyn SecretStore>, base: &str) -> Self {
        Self {
            store,
            origin: Mutex::new(origin(base).unwrap_or_default()),
        }
    }
    pub fn new(store: Arc<dyn SecretStore>, base: &str) -> Result<Self, String> {
        let origin = origin(base)?;
        Ok(Self {
            store,
            origin: Mutex::new(origin),
        })
    }
    fn migrate(&self, current: &str) -> Result<(), String> {
        if current.is_empty() {
            return Err("Invalid Integration URL".into());
        }
        if let Some(value) = self.store.legacy()? {
            if self.store.get("cliproxyapi", "management_key")?.is_none() {
                self.store.put(
                    "cliproxyapi",
                    "management_key",
                    Secret {
                        origin: current.to_owned(),
                        value,
                    },
                )?;
            }
            self.store.delete_legacy()?;
        }
        Ok(())
    }
    pub fn rebind(&self, base: &str) -> Result<(), String> {
        let next = origin(base)?;
        let mut current = self.origin.lock().unwrap();
        if current.is_empty() {
            self.store.delete_legacy()?;
        } else {
            self.migrate(&current)?;
        }
        if *current != next {
            self.store.delete("cliproxyapi", "management_key")?;
            *current = next;
        }
        Ok(())
    }
}
impl KeyStore for BoundKeys {
    fn get(&self) -> Result<Option<String>, String> {
        let current = self.origin.lock().unwrap();
        self.migrate(&current)?;
        Ok(self
            .store
            .get("cliproxyapi", "management_key")?
            .filter(|s| s.origin == *current)
            .map(|s| s.value))
    }
    fn get_for(&self, base: &str) -> Result<Option<String>, String> {
        let expected = origin(base)?;
        let current = self.origin.lock().unwrap();
        self.migrate(&current)?;
        Ok(self
            .store
            .get("cliproxyapi", "management_key")?
            .filter(|s| s.origin == expected && expected == *current)
            .map(|s| s.value))
    }
    fn set(&self, value: &str) -> Result<(), String> {
        let current = self.origin.lock().unwrap();
        self.migrate(&current)?;
        self.store.put(
            "cliproxyapi",
            "management_key",
            Secret {
                origin: current.clone(),
                value: value.into(),
            },
        )
    }
    fn clear(&self) -> Result<(), String> {
        let current = self.origin.lock().unwrap();
        self.migrate(&current)?;
        self.store.delete("cliproxyapi", "management_key")
    }
}
