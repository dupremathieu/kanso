//! Shared, server-owned Redmine connection storage.
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::importer::redmine::{canonical_url, Client};

#[derive(Clone, Serialize, Deserialize)]
struct StoredConnection {
    id: String,
    name: String,
    url: String,
    key: String,
    revision: String,
}

#[derive(Clone, Serialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub name: String,
    pub url: String,
}

/// The stored key deliberately cannot be serialized through the public API.
pub struct Connection {
    pub info: ConnectionInfo,
    pub revision: String,
    pub client: Client,
}

pub struct Connections {
    path: PathBuf,
    entries: Mutex<Vec<StoredConnection>>,
    writer: Mutex<()>,
}

impl Connections {
    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            entries: Mutex::new(Vec::new()),
            writer: Mutex::new(()),
        }
    }

    pub fn reload(&self) -> Result<(), &'static str> {
        let loaded = Self::load(self.path.clone())?;
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        *entries = loaded
            .entries
            .into_inner()
            .unwrap_or_else(|e| e.into_inner());
        Ok(())
    }

    /// Call once at startup; malformed files fail closed.
    pub fn load(path: PathBuf) -> Result<Self, &'static str> {
        let entries: Vec<StoredConnection> = match std::fs::read(&path) {
            Ok(data) if data.len() <= 1024 * 1024 => {
                serde_json::from_slice(&data).map_err(|_| "invalid Redmine settings")?
            }
            Ok(_) => return Err("invalid Redmine settings"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err("cannot read Redmine settings"),
        };
        let mut ids = std::collections::HashSet::new();
        if entries.len() > 150
            || entries.iter().any(|e| {
                !crate::ids::is_uuid(&e.id)
                    || !crate::ids::is_uuid(&e.revision)
                    || !valid_name(&e.name)
                    || canonical_url(&e.url).ok().as_deref() != Some(&e.url)
                    || Client::new(&e.url, &e.key).is_err()
                    || e.name.contains(&e.key)
                    || e.url.contains(&e.key)
                    || !ids.insert(e.id.clone())
            })
        {
            return Err("invalid Redmine settings");
        }
        Ok(Self {
            path,
            entries: Mutex::new(entries),
            writer: Mutex::new(()),
        })
    }

    pub fn list(&self) -> Vec<ConnectionInfo> {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(info)
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Connection> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries.iter().find(|e| e.id == id)?;
        Some(Connection {
            info: info(entry),
            revision: entry.revision.clone(),
            client: Client::new(&entry.url, &entry.key).ok()?,
        })
    }

    pub fn revision_matches(&self, id: &str, revision: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|e| e.id == id && e.revision == revision)
    }

    /// A missing key retains the old one, but changing URL requires a new key.
    pub fn save(
        &self,
        id: Option<&str>,
        name: &str,
        url: &str,
        key: Option<&str>,
    ) -> Result<ConnectionInfo, &'static str> {
        let name = name.trim();
        if !valid_name(name) {
            return Err("invalid connection name");
        }
        let url = canonical_url(url.trim()).map_err(|_| "invalid Redmine URL")?;
        let _writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let index = id
            .map(|id| {
                entries
                    .iter()
                    .position(|e| e.id == id)
                    .ok_or("connection not found")
            })
            .transpose()?;
        if index.is_none() && entries.len() >= 150 {
            return Err("too many connections");
        }
        let existing = index.map(|i| &entries[i]);
        if key.is_none() && existing.is_some_and(|e| e.url != url) {
            return Err("changing URL requires a new key");
        }
        let key = key
            .or_else(|| existing.map(|e| e.key.as_str()))
            .ok_or("API key required")?;
        Client::new(&url, key).map_err(|_| "invalid API key")?;
        if name.contains(key) || url.contains(key) {
            return Err("connection name and URL must not contain the API key");
        }
        let entry = StoredConnection {
            id: existing
                .map(|e| e.id.clone())
                .unwrap_or(crate::ids::uuid_v4().map_err(|_| "random source failed")?),
            name: name.to_owned(),
            url,
            key: key.to_owned(),
            revision: crate::ids::uuid_v4().map_err(|_| "random source failed")?,
        };
        let mut next = entries.clone();
        if let Some(i) = index {
            next[i] = entry.clone();
        } else {
            next.push(entry.clone());
        }
        self.persist(&next)?;
        *self.entries.lock().unwrap_or_else(|e| e.into_inner()) = next;
        Ok(info(&entry))
    }

    pub fn delete(&self, id: &str) -> Result<(), &'static str> {
        let _writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let before = next.len();
        next.retain(|entry| entry.id != id);
        if before == next.len() {
            return Err("connection not found");
        }
        self.persist(&next)?;
        *self.entries.lock().unwrap_or_else(|e| e.into_inner()) = next;
        Ok(())
    }

    fn persist(&self, entries: &[StoredConnection]) -> Result<(), &'static str> {
        let bytes = serde_json::to_vec(entries).map_err(|_| "cannot save Redmine settings")?;
        crate::fileutil::write_atomic(&self.path, &bytes, 0o600)
            .map_err(|_| "cannot save Redmine settings")
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 200 && !name.chars().any(char::is_control)
}

fn info(entry: &StoredConnection) -> ConnectionInfo {
    ConnectionInfo {
        id: entry.id.clone(),
        name: entry.name.clone(),
        url: entry.url.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_rotation_and_write_only_metadata() {
        let dir = crate::test_support::TempDir::new();
        let path = dir.path().join("redmine.json");
        let store = Connections::empty(path.clone());
        let created = store
            .save(
                None,
                "Test",
                "http://EXAMPLE.com:80/redmine/",
                Some("public-test-key"),
            )
            .unwrap();
        assert!(!serde_json::to_string(&store.list())
            .unwrap()
            .contains("public-test-key"));
        let revision = store.get(&created.id).unwrap().revision;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let reopened = Connections::load(path).unwrap();
        assert_eq!(reopened.list()[0].url, "http://example.com/redmine");
        assert!(reopened
            .save(Some(&created.id), "Test", "http://elsewhere", None)
            .is_err());
        reopened
            .save(
                Some(&created.id),
                "Renamed",
                "http://example.com/redmine",
                Some("another-public-key"),
            )
            .unwrap();
        assert!(!reopened.revision_matches(&created.id, &revision));
        reopened.delete(&created.id).unwrap();
        assert!(reopened.list().is_empty());
    }

    #[test]
    fn malformed_and_oversized_files_fail_closed() {
        let dir = crate::test_support::TempDir::new();
        let path = dir.path().join("redmine.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(Connections::load(path.clone()).is_err());
        std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(Connections::load(path).is_err());
    }
}
