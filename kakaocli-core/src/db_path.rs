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

/// macOS: KakaoTalk database directory (Application Support subdirectory)
pub fn mac_db_dir() -> PathBuf {
    mac_container_path()
        .join("Library/Application Support/com.kakao.KakaoTalkMac")
}

/// macOS: find all DB candidate files (78-char hex filenames in Application Support dir)
pub fn mac_db_files() -> Vec<PathBuf> {
    let dir = mac_db_dir();
    if !dir.exists() {
        return vec![];
    }
    std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        // Strip .db extension if present
                        let stem = name.strip_suffix(".db").unwrap_or(name);
                        // 78-char hex filename, not -wal/-shm
                        stem.len() == 78
                            && stem.chars().all(|c| c.is_ascii_hexdigit())
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
            // Brute-force SHA-512 to recover userId.
            // userIds are 8-9 digits (10^7..10^9). Parallel over the full range,
            // ~1-2 min on modern Mac (SHA-512 is fast).
            eprintln!("🔐 SHA-512 userId 역산 시작 (0..1,000,000,000, 멀티스레드)...");
            let start = std::time::Instant::now();
            use ring::digest::{digest, SHA512};
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicU64, Ordering};

            let counter = AtomicU64::new(0);
            let max_id: u64 = 1_000_000_000;

            let found = (0..max_id).into_par_iter().find_map_any(|candidate| {
                let n = counter.fetch_add(1, Ordering::Relaxed);
                if n % 50_000_000 == 0 {
                    let elapsed = start.elapsed().as_secs();
                    eprintln!(
                        "⏳ {}M/1000M 진행 ({}초 경과)...",
                        n / 1_000_000,
                        elapsed
                    );
                }
                let id_str = candidate.to_string();
                let computed = digest(&SHA512, id_str.as_bytes());
                let computed_hex = hex::encode(computed.as_ref());
                if computed_hex == active_hash {
                    Some(candidate as u64)
                } else {
                    None
                }
            });

            if let Some(user_id) = found {
                let elapsed = start.elapsed().as_secs_f64();
                eprintln!("✅ userId = {} ({}초 소요)", user_id, elapsed);
                return Some(user_id);
            }
            eprintln!("❌ 10억 범위 내에서 userId를 찾지 못했습니다.");
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

/// kakaocli config directory (~/.kakaocli)
pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/Shared".into());
    PathBuf::from(home).join(".kakaocli")
}

/// Cached userId file path (~/.kakaocli/user_id)
fn user_id_cache_path() -> PathBuf {
    config_dir().join("user_id")
}

/// Read cached userId from config file
pub fn read_cached_user_id() -> Option<u64> {
    let path = user_id_cache_path();
    let content = std::fs::read_to_string(&path).ok()?;
    content.trim().parse::<u64>().ok()
}

/// Write userId to config file (best-effort)
pub fn write_cached_user_id(user_id: u64) {
    let dir = config_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        // Best-effort; ignore write failures (cache is optional)
        let _ = std::fs::write(user_id_cache_path(), user_id.to_string());
    }
}

/// Clear cached userId
pub fn clear_cached_user_id() {
    let _ = std::fs::remove_file(user_id_cache_path());
}

/// Windows: `%LOCALAPPDATA%\Kakao\KakaoTalk` base directory.
pub fn windows_kakao_base() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| r"C:\Users\Default\AppData\Local".into());
    PathBuf::from(local_app_data).join(r"Kakao\KakaoTalk")
}

/// Windows: per-user directory `...\KakaoTalk\users\<40hex>`.
///
/// KakaoTalk 26.x stores each logged-in account under a 40-hex (SHA-1) subdir of
/// `users\`, identified by a `keystore.bin` file. Returns the first such dir.
pub fn windows_user_dir() -> Option<PathBuf> {
    let users = windows_kakao_base().join("users");
    let entries = std::fs::read_dir(&users).ok()?;
    let mut candidate = None;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.join("keystore.bin").exists() {
                return Some(p); // definitive
            }
            if candidate.is_none() && p.join("chat_data").is_dir() {
                candidate = Some(p); // fallback: has chat_data
            }
        }
    }
    candidate
}

/// Windows: KakaoTalk `chat_data` directory (under the per-user dir on 26.x).
/// Falls back to the legacy flat `...\KakaoTalk\chat_data` when no user dir is found.
pub fn windows_chat_data_path() -> PathBuf {
    match windows_user_dir() {
        Some(dir) => dir.join("chat_data"),
        None => windows_kakao_base().join("chat_data"),
    }
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