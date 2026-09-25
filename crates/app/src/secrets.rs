use service::secrets::SecretStore;

const SERVICE: &str = "costlytics";

/// Access-key secrets in the OS keychain (Windows Credential Manager /
/// macOS Keychain), account `source:<id>`.
pub struct KeyringSecretStore;

fn entry(source_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("source:{source_id}")).map_err(|e| e.to_string())
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, source_id: &str) -> Result<Option<String>, String> {
        match entry(source_id)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, source_id: &str, secret: &str) -> Result<(), String> {
        entry(source_id)?.set_password(secret).map_err(|e| e.to_string())
    }

    fn delete(&self, source_id: &str) -> Result<(), String> {
        match entry(source_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
