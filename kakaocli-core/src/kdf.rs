/// PBKDF2-SHA256 key derivation for macOS KakaoTalk.
/// Based on blluv's research: https://gist.github.com/blluv/8418e3ef4f4aa86004657ea524f2de14
/// Ported from kakaocli (Swift) KeyDerivation.swift
///
/// ## Key format
/// KDF outputs 128 bytes via PBKDF2-HMAC-SHA256 (100,000 iterations).
/// Only the **first 32 bytes** are used as the raw SQLCipher key.
/// This must be passed as a hex-encoded raw key via:
///   PRAGMA key = "x'<64-char-hex>'"
/// NOT as:
///   PRAGMA key = '<passphrase>'  ← this re-derives the key via SQLCipher's KDF and fails!

use ring::pbkdf2;
use std::num::NonZeroU32;

const PBKDF2_ITERATIONS: NonZeroU32 =
    unsafe { NonZeroU32::new_unchecked(100_000) };
const PBKDF2_OUTPUT_LEN: usize = 128;  // 128 bytes

/// Derive hex-encoded raw SQLCipher key (64 hex chars = 32 bytes).
///
/// Algorithm (identical to kakaocli's KeyDerivation.secureKey):
/// 1. Hash device UUID with SHA-1 + SHA-256, base64 encode
/// 2. Build password string from userId + UUID + fixed salts
/// 3. PBKDF2-HMAC-SHA256, 100k iterations → 128 bytes
/// 4. Take first 32 bytes → hex encode
pub fn derive_mac_key(user_id: u64, device_uuid: &str) -> String {
    let hashed = hashed_device_uuid(device_uuid);
    let uuid_str = device_uuid.to_string();
    let user_str = user_id.to_string();

    // Build password: reversed hakawai string
    let parts = [
        "A", &hashed, "|",
        "F", &uuid_str[..5.min(uuid_str.len())],
        "H", &user_str, "|",
        &uuid_str[7.min(uuid_str.len())..],
    ];
    let hawawa = parts.join("F");

    // Salt: last 70% of UUID
    let salt_start = (uuid_str.len() as f64 * 0.3).ceil() as usize;
    let salt = &uuid_str[salt_start..];

    // PBKDF2
    let mut output = [0u8; PBKDF2_OUTPUT_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        PBKDF2_ITERATIONS,
        salt.as_bytes(),
        hawawa.as_bytes(),
        &mut output,
    );

    // First 32 bytes → hex
    hex::encode(&output[..32])
}

/// Derive the encrypted database filename (hex string, no extension).
pub fn derive_mac_db_name(user_id: u64, device_uuid: &str) -> String {
    let uuid_str = device_uuid.to_string();
    let user_str = user_id.to_string();
    let reversed_uuid: String = uuid_str.chars().rev().collect();

    let parts = [
        ".", "F", &user_str, "A", "F",
        &reversed_uuid, ".", "|",
    ];
    let hawawa = parts.join(".");

    let hashed_rev: String = hashed_device_uuid(device_uuid).chars().rev().collect();
    let salt = &hashed_rev;

    let mut output = [0u8; PBKDF2_OUTPUT_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        PBKDF2_ITERATIONS,
        salt.as_bytes(),
        hawawa.as_bytes(),
        &mut output,
    );

    let hex_full = hex::encode(output);
    // Take bytes [14..53] (kakaocli uses: start=28, end=78 in hex = 14, 53 in byte indices)
    let start = 28; // hex chars
    let end = 78;
    hex_full[start..end].to_string()
}

/// SHA-1 + SHA-256 of UUID, base64-encoded (kakaocli compatible)
fn hashed_device_uuid(uuid: &str) -> String {
    use sha1::{Digest, Sha1};
    use sha2::Sha256;

    let data = uuid.as_bytes();

    let sha1 = Sha1::digest(data);
    let sha256 = Sha256::digest(data);

    let combined = [sha1.as_slice(), sha256.as_slice()].concat();
    base64::encode(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_key_deterministic() {
        // Must produce same output for same inputs
        let key1 = derive_mac_key(12345, "test-uuid-0000");
        let key2 = derive_mac_key(12345, "test-uuid-0000");
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_different_user_different_key() {
        let key1 = derive_mac_key(100, "same-uuid");
        let key2 = derive_mac_key(200, "same-uuid");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_db_name_deterministic() {
        let name1 = derive_mac_db_name(12345, "test-uuid");
        let name2 = derive_mac_db_name(12345, "test-uuid");
        assert_eq!(name1, name2);
    }
}