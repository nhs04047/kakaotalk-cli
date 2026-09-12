//! kakaocli-auth: Credential storage (macOS Keychain / Windows Credential Manager).
//!
//! Uses the `keyring` crate to store credentials in the OS keychain:
//! - macOS: Keychain (service = "com.kakaocli-rs.credentials")
//! - Windows: Credential Manager
//! - Linux: Secret Service (if available; falls back to encrypted file)

use keyring::Entry;
use std::sync::LazyLock;

const SERVICE_NAME: &str = "com.kakaocli-rs.credentials";
const EMAIL_ACCOUNT: &str = "kakaotalk-email";
const PASSWORD_ACCOUNT: &str = "kakaotalk-password";

static EMAIL_ENTRY: LazyLock<Entry> = LazyLock::new(|| {
    Entry::new(SERVICE_NAME, EMAIL_ACCOUNT).expect("Failed to create keyring entry")
});

static PASSWORD_ENTRY: LazyLock<Entry> = LazyLock::new(|| {
    Entry::new(SERVICE_NAME, PASSWORD_ACCOUNT).expect("Failed to create keyring entry")
});

/// Store KakaoTalk email and password in the OS keychain.
pub fn store_credentials(email: &str, password: &str) -> Result<(), Box<dyn std::error::Error>> {
    EMAIL_ENTRY.set_password(email)?;
    PASSWORD_ENTRY.set_password(password)?;
    Ok(())
}

/// Retrieve stored credentials. Returns (email, password).
pub fn get_credentials() -> Result<(String, String), Box<dyn std::error::Error>> {
    let email = EMAIL_ENTRY.get_password()?;
    let password = PASSWORD_ENTRY.get_password()?;
    Ok((email, password))
}

/// Remove stored credentials from the OS keychain.
pub fn clear_credentials() -> Result<(), Box<dyn std::error::Error>> {
    let _ = EMAIL_ENTRY.delete_credential();
    let _ = PASSWORD_ENTRY.delete_credential();
    Ok(())
}

/// Check whether credentials are stored in the OS keychain.
pub fn has_credentials() -> bool {
    EMAIL_ENTRY.get_password().is_ok()
}
