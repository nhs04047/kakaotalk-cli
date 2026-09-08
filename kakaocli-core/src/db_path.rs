/// macOS/Windows KakaoTalk database path discovery, user ID extraction, and TCC checks.
///
/// # Platform safety
/// - macOS-specific functions use `#[cfg(target_os = "macos")]` where they depend
///   on macOS-only APIs (accessibility).
/// - Path functions work on all platforms (returns correct paths for the target OS).
/// - Linux compiles but only `mac_container_path()` and `windows_chat_data_path()`
///   format tests are meaningful.

use std::path::PathBuf;

/// macOS: KakaoTalk container directory
pub fn mac_container_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/Shared".into());
    PathBuf::from(home)
        .join("Library/Containers/com.kakao.KakaoTalkMac/Data")
}

/// macOS: find all DB candidate files in container (hex filenames, no extension)
pub fn mac_db_files() -> Vec<PathBuf> {
    let container = mac_container_path();
    if !container.exists() {
        return vec![];
    }
    std::fs::read_dir(&container)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        // Hex filename (50+ chars from DB name) and not -wal/-shm
                        name.len() >= 50
                            && name.chars().all(|c| c.is_ascii_hexdigit())
                            && !name.ends_with("-wal")
                            && !name.ends_with("-shm")
                    } else {
                        false
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// macOS: extract user ID from FSChatWindowTransparency preferences plist
///
/// KakaoTalk stores user ID in:
///   ~/Library/Preferences/com.kakao.KakaoTalkMac.plist
/// Key: `FSChatWindowTransparency` → value format: `"ChatRoom_<userId>_..."`
pub fn mac_user_id() -> Option<u64> {
    let prefs_path = {
        let home = std::env::var("HOME").ok()?;
        PathBuf::from(home)
            .join("Library/Preferences/com.kakao.KakaoTalkMac.plist")
    };

    if !prefs_path.exists() {
        return None;
    }

    // Parse binary plist with `plist` crate
    let plist_value: plist::Value = plist::from_file(&prefs_path).ok()?;
    let dict = plist_value.into_dictionary()?;
    let transparency_key = dict.get("FSChatWindowTransparency")?;

    if let Some(s) = transparency_key.as_string() {
        // Format: "ChatRoom_123456_..."
        if let Some(id_part) = s.split('_').nth(1) {
            return id_part.parse::<u64>().ok();
        }
    }

    None
}

/// macOS: get platform UUID from IOPlatformExpertDevice (ioreg)
#[cfg(target_os = "macos")]
pub fn mac_platform_uuid() -> Option<String> {
    let output = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("IOPlatformUUID") {
            // "IOPlatformUUID" = "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
            if let Some(start) = line.find('"') {
                let rest = &line[start + 1..];
                if let Some(end) = rest.find('"') {
                    let uuid = &rest[..end];
                    if uuid.len() == 36 && uuid.chars().filter(|&c| c == '-').count() == 4 {
                        return Some(uuid.to_string());
                    }
                }
            }
        }
    }
    None
}

/// macOS: get platform UUID stub for non-macOS (always returns None)
///
/// This allows the function to be called from platform-agnostic code;
/// the real implementation only exists on macOS.
#[cfg(not(target_os = "macos"))]
pub fn mac_platform_uuid() -> Option<String> {
    None
}

/// macOS: check Full Disk Access permission (can we read ~/Library/Containers?)
pub fn check_full_disk_access() -> bool {
    mac_container_path().exists()
}

/// macOS: check Accessibility permission (can we interact with other apps' AX?)
///
/// Check if Accessibility permission is granted by calling
/// ApplicationServices' AXIsProcessTrusted() (built-in macOS framework,
/// no extra crate needed). On non-macOS, always returns `false`.
#[cfg(target_os = "macos")]
pub fn check_accessibility_permission() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> std::os::raw::c_int;
    }
    // SAFETY: AXIsProcessTrusted is a simple getter with no side effects.
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Stub for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub fn check_accessibility_permission() -> bool {
    false
}

/// Windows: KakaoTalk chat_data directory
pub fn windows_chat_data_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| r"C:\Users\Default\AppData\Local".into());
    PathBuf::from(local_app_data)
        .join(r"Kakao\KakaoTalk\chat_data")
}

/// Windows: find all .edb files in chat_data
pub fn windows_edb_files() -> Vec<PathBuf> {
    let base = windows_chat_data_path();
    if !base.exists() {
        return vec![];
    }
    std::fs::read_dir(&base)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map(|ext| ext == "edb")
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mac_container_path_format() {
        let path = mac_container_path();
        let s = path.to_string_lossy();
        assert!(s.contains("KakaoTalkMac"));
        assert!(s.contains("Containers"));
    }

    #[test]
    fn test_windows_path_format() {
        let path = windows_chat_data_path();
        let s = path.to_string_lossy();
        assert!(s.contains("Kakao"));
        assert!(s.contains("chat_data"));
    }

    #[test]
    fn test_check_accessibility_permission_exists() {
        // Should always return a bool without panicking
        let _result = check_accessibility_permission();
    }
}