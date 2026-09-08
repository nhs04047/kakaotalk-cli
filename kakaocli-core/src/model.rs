use serde::{Deserialize, Serialize};

/// KakaoTalk chat room
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    pub chat_type: ChatType,
    pub display_name: String,
    pub member_count: i32,
    pub last_message_id: Option<i64>,
    pub last_message_at: Option<i64>,
    pub unread_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChatType {
    Direct,
    Group,
    Open,
    SelfChat,
    Unknown(i32),
}

impl ChatType {
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            0 => Self::Direct,
            1 => Self::Group,
            5 => Self::SelfChat,
            _ => Self::Unknown(raw),
        }
    }
}

/// KakaoTalk message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    pub sender_name: Option<String>,
    pub text: Option<String>,
    pub message_type: MessageType,
    pub created_at: i64,
    pub is_from_me: bool,
}

/// Message type — must use `#[repr(i32)]` + manual From<i32>
/// (Rust enum variants cannot have discriminants with serde derive)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[repr(i32)]
pub enum MessageType {
    Text = 1,
    Photo = 2,
    Video = 3,
    Unknown(i32),
}

impl From<i32> for MessageType {
    fn from(value: i32) -> Self {
        match value {
            1 => Self::Text,
            2 => Self::Photo,
            3 => Self::Video,
            other => Self::Unknown(other),
        }
    }
}

/// Application status result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub status: String,
    pub db_found: bool,
    pub db_decrypted: bool,
    pub kakao_talk_version: Option<String>,
}

/// Result of DB key derivation and decryption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub user_id: Option<i64>,
    pub device_uuid: Option<String>,
    pub db_name: Option<String>,
    pub db_path: Option<String>,
    pub key_valid: bool,
    pub tables_found: Vec<String>,
}

/// Database key (macOS: derived, Windows: scanned from process memory)
#[derive(Debug, Clone)]
pub struct DbKey {
    pub key_hex: String,  // 64-char hex string
    pub db_path: std::path::PathBuf,
}