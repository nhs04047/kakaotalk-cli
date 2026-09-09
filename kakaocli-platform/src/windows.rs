// Windows backend: DEK process memory scan + UIAutomation
// References:
// - MoniKa (https://github.com/maxswjeon/MoniKa)
// - windows-rs crate docs

use crate::*;
use kakaocli_core::db_path;
use kakaocli_core::model::DbKey;

pub struct WindowsBackend;

impl PlatformBackend for WindowsBackend {
    fn resolve_db_key(_user_id: Option<u64>) -> Result<DbKey, PlatformError> {
        // TODO: DEK scanner via windows-rs
        // 1. Find KakaoTalk.exe PID (CreateToolhelp32Snapshot)
        // 2. OpenProcess(PROCESS_VM_READ)
        // 3. Scan heap for 32-byte DEK candidates
        // 4. Verify each candidate with BCrypt AES-256 page-1 oracle
        // 5. Cache verified DEK

        let db_path = db_path::windows_edb_files()
            .first()
            .cloned()
            .ok_or_else(|| PlatformError::Other("No .edb files found in chat_data".into()))?;

        Err(PlatformError::Other(
            "Windows DEK scanner not yet implemented".into(),
        ))
    }

    fn check_status() -> Result<AppStatus, PlatformError> {
        // Check if KakaoTalk.exe is running
        // Check if .edb files exist
        let edb_exists = db_path::windows_edb_files().first().is_some();

        if !edb_exists {
            return Ok(AppStatus::NotRunning);
        }

        // TODO: check if KakaoTalk.exe process exists
        Ok(AppStatus::DbAccessible) // placeholder
    }

    fn login(email: &str, password: &str) -> Result<(), PlatformError> {
        kakaocli_auth::store_credentials(email, password)
            .map_err(|e| PlatformError::Other(e.to_string()))
    }

    fn send_message(_chat_name: &str, _text: &str) -> Result<(), PlatformError> {
        // TODO: Implement UIAutomation send flow
        // 1. FindWindow → KakaoTalk main window
        // 2. IUIAutomation Tree → find chat ListItem by name
        // 3. InvokePattern → open chat
        // 4. Verify opened chat title matches expected name
        // 5. SendKeys for text input
        // 6. InvokePattern → send button

        Err(PlatformError::Other("Windows send not yet implemented".into()))
    }

    fn dump_ax_tree(_chat: Option<&str>, _max_depth: u32) -> Result<AxNode, PlatformError> {
        Err(PlatformError::Other(
            "inspect is only supported on macOS".into(),
        ))
    }
}