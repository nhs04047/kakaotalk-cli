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

/// KakaoTalk chat type
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

/// Message type — `#[repr(i32)]` + manual `From<i32>` for DB integer mapping.
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

/// KakaoTalk friend (NTUser)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friend {
    pub user_id: i64,
    pub nick_name: Option<String>,
    pub friend_nick_name: Option<String>,
    pub display_name: Option<String>,
}

/// Unified search results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub messages: Vec<Message>,
    pub rooms: Vec<Chat>,
    pub friends: Vec<Friend>,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── ChatType ──────────────────────────────────────────

    #[test]
    fn test_chat_type_from_raw() {
        assert_eq!(ChatType::from_raw(0), ChatType::Direct);
        assert_eq!(ChatType::from_raw(1), ChatType::Group);
        assert_eq!(ChatType::from_raw(5), ChatType::SelfChat);
        assert_eq!(ChatType::from_raw(99), ChatType::Unknown(99));
    }

    #[test]
    fn test_chat_type_serde_roundtrip() {
        let cases = vec![
            ChatType::Direct,
            ChatType::Group,
            ChatType::SelfChat,
            ChatType::Unknown(42),
        ];
        for ct in cases {
            let json = serde_json::to_string(&ct).unwrap();
            let back: ChatType = serde_json::from_str(&json).unwrap();
            assert_eq!(ct, back);
        }
    }

    // ── MessageType ───────────────────────────────────────

    #[test]
    fn test_message_type_from_i32() {
        assert_eq!(MessageType::from(1), MessageType::Text);
        assert_eq!(MessageType::from(2), MessageType::Photo);
        assert_eq!(MessageType::from(3), MessageType::Video);
        assert_eq!(MessageType::from(0), MessageType::Unknown(0));
        assert_eq!(MessageType::from(999), MessageType::Unknown(999));
    }

    #[test]
    fn test_message_type_serde_roundtrip() {
        let cases = vec![
            MessageType::Text,
            MessageType::Photo,
            MessageType::Video,
            MessageType::Unknown(7),
        ];
        for mt in cases {
            let json = serde_json::to_string(&mt).unwrap();
            let back: MessageType = serde_json::from_str(&json).unwrap();
            assert_eq!(mt, back);
        }
    }

    // ── Chat ──────────────────────────────────────────────

    #[test]
    fn test_chat_serde_roundtrip() {
        let chat = Chat {
            id: 12345,
            chat_type: ChatType::Group,
            display_name: "가족".into(),
            member_count: 4,
            last_message_id: Some(999),
            last_message_at: Some(1700000000),
            unread_count: 3,
        };
        let json = serde_json::to_string(&chat).unwrap();
        let back: Chat = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 12345);
        assert_eq!(back.display_name, "가족");
        assert_eq!(back.member_count, 4);
    }

    // ── Message ───────────────────────────────────────────

    #[test]
    fn test_message_serde_roundtrip() {
        let msg = Message {
            id: 5000,
            chat_id: 12345,
            sender_id: 67890,
            sender_name: Some("지수".into()),
            text: Some("안녕!".into()),
            message_type: MessageType::Text,
            created_at: 1700000000,
            is_from_me: false,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sender_name.as_deref(), Some("지수"));
        assert_eq!(back.text.as_deref(), Some("안녕!"));
        assert_eq!(back.message_type, MessageType::Text);
        assert!(!back.is_from_me);
    }

    #[test]
    fn test_message_with_null_sender() {
        let msg = Message {
            id: 5001,
            chat_id: 12345,
            sender_id: 0,
            sender_name: None,
            text: None,
            message_type: MessageType::Unknown(10),
            created_at: 1700000000,
            is_from_me: true,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert!(back.sender_name.is_none());
        assert!(back.text.is_none());
        assert_eq!(back.message_type, MessageType::Unknown(10));
        assert!(back.is_from_me);
    }

    // ── AuthResult ────────────────────────────────────────

    #[test]
    fn test_auth_result_default() {
        let result = AuthResult {
            user_id: None,
            device_uuid: None,
            db_name: None,
            db_path: None,
            key_valid: false,
            tables_found: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("key_valid"));
        assert!(json.contains("false"));
    }

    #[test]
    fn test_auth_result_with_tables() {
        let result = AuthResult {
            user_id: Some(12345),
            device_uuid: Some("ABCD".into()),
            db_name: Some("abcd1234".into()),
            db_path: Some("/path/to/db".into()),
            key_valid: true,
            tables_found: vec!["NTChatRoom".into(), "NTChatMessage".into()],
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: AuthResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.user_id, Some(12345));
        assert_eq!(back.tables_found.len(), 2);
        assert!(back.key_valid);
    }

    // ── AppInfo ───────────────────────────────────────────

    #[test]
    fn test_app_info_serde() {
        let info = AppInfo {
            status: "ready".into(),
            db_found: true,
            db_decrypted: true,
            kakao_talk_version: Some("3.5.4".into()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: AppInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, "ready");
        assert!(back.db_found);
        assert!(back.db_decrypted);
        assert_eq!(back.kakao_talk_version.as_deref(), Some("3.5.4"));
    }
}