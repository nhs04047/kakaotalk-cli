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

/// macOS: extract user ID from KakaoTalk preferences plist
///
/// KakaoTalk stores user ID obfuscated in the container-scoped preferences plist.
/// Strategy: find DESIGNATEDFRIENDSREVISION:<sha512> with non-zero value
/// (= active account), then brute-force SHA-512 to find the matching userId.
pub fn mac_user_id() -> Option<u64> {
    let plist_candidates = mac_plist_candidates();

    for plist_path in &plist_candidates {
        if !plist_path.exists() {
            continue;
        }
        let plist_value: plist::Value = match plist::from_file(plist_path).ok() {
            Some(v) => v,
            None => continue,
        };
        let dict = match plist_value.into_dictionary() {
            Some(d) => d,
            None => continue,
        };

        // Find active account SHA-512 hash from DESIGNATEDFRIENDSREVISION:<sha512> keys
        let hash_prefix = "DESIGNATEDFRIENDSREVISION:";
        let empty_hash = "31bca02094eb78126a517b206a88c73cfa9ec6f704c7030d18212cace820f025f00bf0ea68dbf3f3a5436ca63b53bf7bf80ad8d5de7d8359d0b7fed9dbc3ab99";
        let active_hash: Option<String> = dict.iter().find_map(|(key, val)| {
            if key.starts_with(hash_prefix) {
                let hash = &key[hash_prefix.len()..];
                if hash == empty_hash {
                    return None;
                }
                let is_nonzero = val
                    .as_unsigned_integer()
                    .map(|n| n > 0)
                    .unwrap_or_else(|| val.as_real().map(|f| f != 0.0).unwrap_or(false));
                if is_nonzero {
                    Some(hash.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        });

        if let Some(active_hash) = active_hash {
            // Brute-force SHA-512 from 0 upward until we find a match.
            // userIds are typically small integers (< 1M).
            use ring::digest::{digest, SHA512};
            let max_id = 1_000_000;
            for candidate in 0..max_id {
                let id_str = candidate.to_string();
                let computed = digest(&SHA512, id_str.as_bytes());
                let computed_hex = hex::encode(computed.as_ref());
                if computed_hex == active_hash {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

/// 모든 plist 후보를 크기 내림차순으로 반환 (빈 plist를 나중에 보게)
fn mac_plist_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // Container hex-suffix plists
    let container = mac_container_path();
    let prefs_dir = container.join("Library/Preferences");
    if let Ok(entries) = std::fs::read_dir(&prefs_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name_str) = name.to_str() {
                if name_str.starts_with("com.kakao.KakaoTalkMac.")
                    && name_str.ends_with(".plist")
                    && name_str != "com.kakao.KakaoTalkMac.plist"
                {
                    candidates.push(prefs_dir.join(name_str));
                }
            }
        }
    }

    // Container base plist
    candidates.push(prefs_dir.join("com.kakao.KakaoTalkMac.plist"));

    // Global plist
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/Shared".into());
    candidates.push(PathBuf::from(home).join("Library/Preferences/com.kakao.KakaoTalkMac.plist"));

    // Sort by file size descending (largest first = most data)
    candidates.sort_by(|a, b| {
        let a_size = std::fs::metadata(a).map(|m| m.len()).unwrap_or(0);
        let b_size = std::fs::metadata(b).map(|m| m.len()).unwrap_or(0);
        b_size.cmp(&a_size)
    });

    candidates
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