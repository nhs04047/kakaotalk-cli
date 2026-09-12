/// PBKDF2-SHA256 key derivation for macOS KakaoTalk.
/// Based on blluv's research: https://gist.github.com/blluv/8418e3ef4f4aa86004657ea524f2de14
/// Ported from kakaocli (Swift) KeyDerivation.swift
///
/// ## Key format
/// KDF outputs 128 bytes via PBKDF2-HMAC-SHA256 (100,000 iterations).
/// The FULL 128 bytes are hex-encoded (256 chars) and used as a **passphrase**:
///   PRAGMA key = '<256-char-hex>'
/// (kakaocli Swift: `PRAGMA KEY='<hex>'` — SQLCipher re-derives internally.)
/// Do NOT truncate to 32 bytes — that would yield a different key.
use ring::digest::{digest, SHA1_FOR_LEGACY_USE_ONLY, SHA256};
use ring::pbkdf2;
use std::num::NonZeroU32;

/// PBKDF2 iterations: 100,000 (matches kakaocli)
const PBKDF2_ITERATIONS: NonZeroU32 = NonZeroU32::new(100_000).expect("100k is non-zero");
/// PBKDF2 output length in bytes (kakaocli uses 128)
const PBKDF2_OUTPUT_LEN: usize = 128;

/// SHA-1 digest output length
const SHA1_LEN: usize = 20;
/// SHA-256 digest output length
const SHA256_LEN: usize = 32;

/// Derive hex-encoded SQLCipher passphrase key (256 hex chars = 128 bytes).
///
/// Algorithm (identical to kakaocli's KeyDerivation.secureKey):
/// 1. Hash device UUID with SHA-1 + SHA-256 → concatenate → base64 encode
/// 2. Build password string from userId + UUID + fixed salts
/// 3. PBKDF2-HMAC-SHA256, 100k iterations → 128 bytes
/// 4. Hex-encode the FULL 128 bytes → used as passphrase via `PRAGMA key = '...'`
///
/// NOTE: kakaocli (Swift) uses `PRAGMA KEY='<hex>'` which makes SQLCipher
/// re-derive the key (passphrase KDF). The FULL 128-byte hex must be used,
/// NOT just the first 32 bytes.
pub fn derive_mac_key(user_id: u64, device_uuid: &str) -> String {
    let hashed = hashed_device_uuid(device_uuid);
    let uuid_str = device_uuid.to_string();
    let user_str = user_id.to_string();

    // Build password: A + hashed_uuid + "|" + F + uuid[:5] + H + userId + "|" + uuid[7..]
    // joined with "F" between each part, then reversed
    let parts = [
        "A",
        &hashed,
        "|",
        "F",
        &uuid_str[..5.min(uuid_str.len())],
        "H",
        &user_str,
        "|",
        &uuid_str[7.min(uuid_str.len())..],
    ];
    let hawawa = parts.join("F");
    let hawawa_rev: String = hawawa.chars().rev().collect();

    // Salt: last ~70% of UUID — blluv 원본과 동일하게 int() = 버림
    // Python: uuid[int(len(uuid) * 0.3):]  (36자 × 0.3 = 10.8 → 10)
    let salt_start = (uuid_str.len() as f64 * 0.3) as usize;
    let salt = &uuid_str[salt_start..];

    // PBKDF2
    let mut output = [0u8; PBKDF2_OUTPUT_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        PBKDF2_ITERATIONS,
        salt.as_bytes(),
        hawawa_rev.as_bytes(),
        &mut output,
    );

    // Full 128 bytes → hex (256 chars) — passphrase for SQLCipher
    // (kakaocli Swift: derived.map { String(format: "%02x", $0) }.joined())
    hex::encode(output)
}

/// Derive the encrypted database filename (hex string, no extension).
pub fn derive_mac_db_name(user_id: u64, device_uuid: &str) -> String {
    let uuid_str = device_uuid.to_string();
    let user_str = user_id.to_string();
    let reversed_uuid: String = uuid_str.chars().rev().collect();

    // Build password: . + F + userId + A + F + reversed_uuid + . + "|"
    // joined with "." between each part
    let parts = [".", "F", &user_str, "A", "F", &reversed_uuid, ".", "|"];
    let hawawa = parts.join(".");

    // Salt: reversed hashed device UUID
    let hashed_rev: String = hashed_device_uuid(device_uuid).chars().rev().collect();

    let mut output = [0u8; PBKDF2_OUTPUT_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        PBKDF2_ITERATIONS,
        hashed_rev.as_bytes(),
        hawawa.as_bytes(),
        &mut output,
    );

    let hex_full = hex::encode(output);
    // Take hex chars [28..106) — kakaocli Swift: start=28, offsetBy=78 → 78 chars
    hex_full[28..106].to_string()
}

/// SHA-1 + SHA-256 of UUID, base64-encoded (kakaocli compatible)
///
/// Equivalent to Swift:
/// ```swift
/// CC_SHA1(data, len, sha1_ptr)
/// CC_SHA256(data, len, sha256_ptr)
/// return (sha1 + sha256).base64EncodedString()
/// ```
fn hashed_device_uuid(uuid: &str) -> String {
    let data = uuid.as_bytes();

    let sha1_result = digest(&SHA1_FOR_LEGACY_USE_ONLY, data);
    let sha256_result = digest(&SHA256, data);

    // Concatenate SHA-1 (20 bytes) + SHA-256 (32 bytes)
    let mut combined = Vec::with_capacity(SHA1_LEN + SHA256_LEN);
    combined.extend_from_slice(sha1_result.as_ref());
    combined.extend_from_slice(sha256_result.as_ref());

    use base64::engine::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_key_deterministic() {
        let key1 = derive_mac_key(12345, "test-uuid-0000");
        let key2 = derive_mac_key(12345, "test-uuid-0000");
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 256); // 128 bytes = 256 hex chars (full PBKDF2 output)
    }

    #[test]
    fn test_different_user_different_key() {
        let key1 = derive_mac_key(100, "same-uuid");
        let key2 = derive_mac_key(200, "same-uuid");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_different_uuid_different_key() {
        let key1 = derive_mac_key(100, "uuid-AAAA");
        let key2 = derive_mac_key(100, "uuid-BBBB");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_key_length_is_64_hex_chars() {
        let key = derive_mac_key(42, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(key.len(), 256);
        // Verify it's valid hex
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_db_name_deterministic() {
        let name1 = derive_mac_db_name(12345, "test-uuid");
        let name2 = derive_mac_db_name(12345, "test-uuid");
        assert_eq!(name1, name2);
    }

    #[test]
    fn test_db_name_length() {
        let name = derive_mac_db_name(42, "550e8400-e29b-41d4-a716-446655440000");
        // kakaocli: start=28, offsetBy=78 → hex[28..106] = 78 hex chars = 39 bytes
        assert_eq!(name.len(), 78);
    }

    #[test]
    fn test_db_name_different_user() {
        let name1 = derive_mac_db_name(100, "same-uuid");
        let name2 = derive_mac_db_name(200, "same-uuid");
        assert_ne!(name1, name2);
    }

    #[test]
    fn test_hashed_device_uuid_output_length() {
        let hash = hashed_device_uuid("test-uuid");
        // SHA-1 (20) + SHA-256 (32) = 52 bytes → base64 = 52 * 4/3 ≈ 70 chars (with padding)
        assert!(!hash.is_empty());
        assert!(hash.len() >= 68); // base64 encoded length
    }

    #[test]
    fn test_hashed_device_uuid_deterministic() {
        let h1 = hashed_device_uuid("550e8400-e29b-41d4-a716-446655440000");
        let h2 = hashed_device_uuid("550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_reverse_password_string() {
        // Verify the "reversed" aspect of derive_mac_key
        let key = derive_mac_key(42, "short");
        // Just verify it doesn't panic with short UUIDs
        assert_eq!(key.len(), 256);
    }

    #[test]
    fn test_salt_truncation_matches_blluv() {
        // blluv 원본: salt = uuid[int(len(uuid) * 0.3):]
        // 36자 UUID → 36 * 0.3 = 10.8 → int() = 10 (버림, ceil 아님!)
        let uuid = "1591D3B8-9C8A-4F1C-5620-ABCDEF123456"; // 36 chars
        assert_eq!(uuid.len(), 36);

        let salt_start = (uuid.len() as f64 * 0.3) as usize;
        assert_eq!(salt_start, 10); // ceil이면 11 — 버그 재발 방지

        let salt = &uuid[salt_start..];
        assert_eq!(salt, &uuid[10..]);
        assert_eq!(salt.len(), 26);
    }
}
