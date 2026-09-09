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

/// macOS: extract user ID from KakaoTalk preferences plist (kakaocli 방식)
///
/// KakaoTalk stores user ID in the container-scoped preferences plist.
/// Multiple extraction strategies are tried in order:
/// 1. FSChatWindowTransparency 공통 접미사 (키 이름에서 추출)
/// 2. Direct key lookup (userId, user_id, KAKAO_USER_ID, userID)
/// 3. NSWindow Frame FSChatWindowFrame_ 공통 접미사
pub fn mac_user_id() -> Option<u64> {
    // 1. Find the right plist — container plist preferred (may have hex suffix)
    let plist_candidates = vec![
        mac_container_user_prefs_path(),
        mac_global_prefs_path(),
    ];

    for plist_path in plist_candidates {
        if !plist_path.exists() {
            continue;
        }

        let plist_value: plist::Value = plist::from_file(&plist_path).ok()?;
        let dict = plist_value.into_dictionary()?;

        // Strategy 1: FSChatWindowTransparency 공통 접미사
        let transparency_prefix = "FSChatWindowTransparency";
        let fs_chat_keys: Vec<&str> = dict.keys()
            .filter(|k| k.starts_with(transparency_prefix))
            .map(|k| k.as_str())
            .collect();
        if fs_chat_keys.len() >= 2 {
            let suffixes: Vec<&str> = fs_chat_keys.iter()
                .map(|k| &k[transparency_prefix.len()..])
                .collect();
            if let Some(common) = longest_common_suffix(&suffixes) {
                if let Ok(id) = common.parse::<u64>() {
                    return Some(id);
                }
            }
        }

        // Strategy 2: Direct key lookup
        let candidate_keys = ["userId", "user_id", "KAKAO_USER_ID", "userID"];
        for key in &candidate_keys {
            if let Some(val) = dict.get(*key) {
                if let Some(n) = val.as_unsigned_integer() {
                    if n > 0 {
                        return Some(n);
                    }
                }
                if let Some(s) = val.as_string() {
                    if let Ok(n) = s.parse::<u64>() {
                        return Some(n);
                    }
                }
            }
        }

        // Strategy 3: NSWindow Frame FSChatWindowFrame_ 공통 접미사
        let frame_prefix = "NSWindow Frame FSChatWindowFrame_";
        let frame_keys: Vec<&str> = dict.keys()
            .filter(|k| k.starts_with(frame_prefix))
            .map(|k| k.as_str())
            .collect();
        if frame_keys.len() >= 2 {
            let suffixes: Vec<&str> = frame_keys.iter()
                .map(|k| &k[frame_prefix.len()..])
                .collect();
            if let Some(common) = longest_common_suffix(&suffixes) {
                if let Ok(id) = common.parse::<u64>() {
                    return Some(id);
                }
            }
        }
    }

    None
}

/// 컨테이너 내부의 사용자 preferences plist 경로 (hex-suffix plist 우선)
fn mac_container_user_prefs_path() -> PathBuf {
    let container = mac_container_path(); // .../Data/
    let prefs_dir = container.join("Library/Preferences");
    if let Ok(entries) = std::fs::read_dir(&prefs_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str() {
                if name_str.starts_with("com.kakao.KakaoTalkMac.")
                    && name_str.ends_with(".plist")
                    && name_str != "com.kakao.KakaoTalkMac.plist"
                {
                    return prefs_dir.join(name_str);
                }
            }
        }
    }
    // Fallback: hex-suffix plist 없으면 기본
    prefs_dir.join("com.kakao.KakaoTalkMac.plist")
}

/// 글로벌 preferences plist 경로
fn mac_global_prefs_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/Shared".into());
    PathBuf::from(home).join("Library/Preferences/com.kakao.KakaoTalkMac.plist")
}

/// 문자열 배열의 공통 접미사 찾기
fn longest_common_suffix(strings: &[&str]) -> Option<String> {
    let first = strings.first()?;
    let reversed: Vec<String> = strings.iter().map(|s| s.chars().rev().collect()).collect();
    let mut common_len = 0usize;
    'outer: for (i, ch) in reversed[0].char_indices() {
        for r in &reversed[1..] {
            if r.chars().nth(i) != Some(ch) {
                break 'outer;
            }
        }
        common_len = i + 1; // char_indices is 0-based, but we want 1-based count
    }
    if common_len == 0 {
        return None;
    }
    Some(first[first.len() - common_len..].to_string())
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
            // Format: "IOPlatformUUID" = "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
            if let Some(equals_pos) = line.find('=') {
                let after_equals = &line[equals_pos + 1..];
                if let Some(start) = after_equals.find('"') {
                    let value_part = &after_equals[start + 1..];
                    if let Some(end) = value_part.find('"') {
                        let uuid = &value_part[..end];
                        if uuid.len() == 36
                            && uuid.chars().filter(|&c| c == '-').count() == 4
                        {
                            return Some(uuid.to_string());
                        }
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