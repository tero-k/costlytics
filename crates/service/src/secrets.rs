use std::collections::HashMap;
use std::sync::Mutex;

/// Where access-key secrets live, keyed by source id. The desktop app uses
/// the OS keychain (`crates/app/src/secrets.rs`); the HTTP harness and tests
/// use [`MemorySecretStore`]. Errors are human-readable strings.
pub trait SecretStore: Send + Sync {
    fn get(&self, source_id: &str) -> Result<Option<String>, String>;
    fn set(&self, source_id: &str, secret: &str) -> Result<(), String>;
    /// Deleting a secret that doesn't exist is not an error.
    fn delete(&self, source_id: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, source_id: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(source_id).cloned())
    }
    fn set(&self, source_id: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(source_id.into(), secret.into());
        Ok(())
    }
    fn delete(&self, source_id: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(source_id);
        Ok(())
    }
}
