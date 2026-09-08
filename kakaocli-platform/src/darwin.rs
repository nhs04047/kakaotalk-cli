// macOS backend: AXUIElement + CGEvent via objc2 + accessibility crate
// References:
// - kakaocli (Swift) CLAUDE.md (AX quirks)
// - accessibility crate docs

use crate::*;
use kakaocli_core::model::DbKey;
use kakaocli_core::db_path;

pub struct DarwinBackend;

impl PlatformBackend for DarwinBackend {
    fn resolve_db_key() -> Result<DbKey, PlatformError> {
        let uuid = db_path::mac_platform_uuid()
            .ok_or_else(|| PlatformError::Other("Cannot read IOPlatformUUID".into()))?;
        let user_id = db_path::mac_user_id()
            .ok_or_else(|| PlatformError::Other("Cannot read userId from plist".into()))?;
        let key_hex = kakaocli_core::kdf::derive_mac_key(user_id, &uuid);

        // Find DB file
        let db_files = db_path::mac_db_files();
        let db_path = db_files.first()
            .cloned()
            .ok_or_else(|| PlatformError::Other("No KakaoTalk database file found".into()))?;

        Ok(DbKey { key_hex, db_path })
    }

    fn check_status() -> Result<AppStatus, PlatformError> {
        if !db_path::check_full_disk_access() {
            return Err(PlatformError::Other(
                "Full Disk Access not granted. Grant it in System Settings > Privacy & Security".into()
            ));
        }

        // Check if KakaoTalk is running
        use objc2::rc::Retained;
        use objc2_app_kit::NSRunningApplication;
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
            "com.kakao.KakaoTalkMac"
        );
        let running = apps.first().is_some();

        let db_exists = db_path::mac_db_files().first().is_some();

        match (running, db_exists) {
            (true, _) => Ok(AppStatus::Ready),
            (false, true) => Ok(AppStatus::DbAccessible),
            (false, false) => Ok(AppStatus::NotRunning),
        }
    }

    fn login(email: &str, password: &str) -> Result<(), PlatformError> {
        kakaocli_auth::store_credentials(email, password)
            .map_err(|e| PlatformError::Other(e.to_string()))
    }

    fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError> {
        // TODO: Implement AXUIElement + CGEvent send flow
        // 1. Ensure KakaoTalk is running (auto-launch if needed, auto-login)
        // 2. Find main window
        // 3. Search chat list for `chat_name`
        // 4. Select chat (AXSelectedRowsAttribute + Enter CGEvent)
        // 5. Verify opened window title matches `chat_name`
        // 6. Type text via CGEventKeyboard (Unicode)
        // 7. Send Enter

        Err(PlatformError::Other("macOS send not yet implemented".into()))
    }
}