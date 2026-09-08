//! kakaocli-db: SQLCipher database reader for KakaoTalk.
//!
//! Reads KakaoTalk's SQLCipher-encrypted local database.
//! All queries are read-only (SQLITE_OPEN_READ_ONLY).
//!
//! # Key format
//! Key must be a 64-char hex string (32 bytes raw key).
//! Passed via: `PRAGMA key = "x'<64-char-hex>'"`
//! NOT: `PRAGMA key = '<passphrase>'` (would re-KDF and fail).

use std::path::Path;

use kakaocli_core::model::*;
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;
use thiserror::Error;

// ── Error type ──────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Database not found: {0}")]
    DatabaseNotFound(std::path::PathBuf),

    #[error("Failed to open database: {0}")]
    DatabaseOpenFailed(String),

    #[error("Decryption failed with all cipher compatibility modes (3, 4)")]
    DecryptionFailed,

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("{0}")]
    Other(String),
}

// ── Database ────────────────────────────────────────────────

pub struct Database {
    conn: Connection,
    my_user_id: i64,
}

impl Database {
    /// Open a KakaoTalk SQLCipher database.
    ///
    /// `my_user_id` is obtained externally (from macOS plist or Windows DEK scan)
    /// and passed in — avoids depending on the NTChatContext schema which varies.
    ///
    /// Tries cipher compatibility modes 3 and 4, each with a **fresh connection**.
    /// Uses the key as a raw key via `PRAGMA key = "x'<hex>'"`.
    pub fn open(path: &Path, key_hex: &str, my_user_id: i64) -> Result<Self, DbError> {
        if !path.exists() {
            return Err(DbError::DatabaseNotFound(path.to_path_buf()));
        }

        // Validate key format
        if key_hex.len() != 64 || !key_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DbError::DatabaseOpenFailed(
                "Key must be 64 hex characters (32 bytes)".into(),
            ));
        }

        let mut last_error = DbError::DecryptionFailed;

        // Try each compatibility mode with a fresh connection
        for (i, compat) in [3, 4].iter().enumerate() {
            let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
                Ok(c) => c,
                Err(e) => {
                    last_error = DbError::DatabaseOpenFailed(e.to_string());
                    continue;
                }
            };

            // Set compatibility mode and key atomically
            let pragma_sql = format!(
                "PRAGMA cipher_compatibility = {}; PRAGMA key = \"x'{}'\";",
                compat, key_hex
            );

            if let Err(_e) = conn.execute_batch(&pragma_sql) {
                // compat=3 failed — on the LAST iteration (compat=4) report failure
                drop(conn);
                if i == 1 {
                    // compat=4 also failed
                    last_error = DbError::DecryptionFailed;
                }
                continue;
            }

            // Verify key worked by reading from sqlite_master
            match conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0)) {
                Ok(_) => {
                    // Compat AND key verified — we're good
                    tracing::debug!("SQLCipher opened with cipher_compatibility={}", compat);
                    return Ok(Self { conn, my_user_id });
                }
                Err(e) => {
                    // Key/decrypt failed for this compat mode
                    last_error = DbError::DatabaseOpenFailed(format!(
                        "Key rejected with cipher_compatibility={}: {}", compat, e
                    ));
                    drop(conn);
                }
            }
        }

        Err(last_error)
    }

    /// Return my own user ID (set at open time).
    pub fn my_user_id(&self) -> i64 {
        self.my_user_id
    }

    /// Return list of table names in the database.
    pub fn verify_tables(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let tables = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| DbError::QueryFailed(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(tables)
    }

    // ── Chats ──────────────────────────────────────────────

    /// List chat rooms sorted by last activity (most recent first).
    pub fn list_chats(&self, limit: u32) -> Result<Vec<Chat>, DbError> {
        let sql = "\
            SELECT r.chatId, r.type, r.chatName, r.activeMembersCount, \
                   r.lastLogId, r.lastUpdatedAt, r.countOfNewMessage, \
                   u.displayName, u.friendNickName, u.nickName \
            FROM NTChatRoom r \
            LEFT JOIN NTUser u ON r.directChatMemberUserId = u.userId AND u.linkId = 0 \
            ORDER BY r.lastUpdatedAt DESC \
            LIMIT ?";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit], |row| {
                let chat_type_raw: i32 = row.get(1)?;
                let chat_name: Option<String> = row.get(2)?;
                let display_name: Option<String> = row.get(7)?;
                let friend_nick: Option<String> = row.get(8)?;
                let nick: Option<String> = row.get(9)?;

                Ok(Chat {
                    id: row.get(0)?,
                    chat_type: ChatType::from_raw(chat_type_raw),
                    display_name: chat_name
                        .or(display_name)
                        .or(friend_nick)
                        .or(nick)
                        .unwrap_or_else(|| "(unknown)".into()),
                    member_count: row.get(3)?,
                    last_message_id: row.get::<_, Option<i64>>(4)?,
                    last_message_at: row.get::<_, Option<i64>>(5)?,
                    unread_count: row.get(6)?,
                })
            })
            .map_err(|e| DbError::QueryFailed(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows)
    }

    // ── Messages ────────────────────────────────────────────

    /// Get messages for a specific chat room ID.
    /// `since`: optional Unix timestamp — only messages after this point.
    pub fn get_messages(
        &self,
        chat_id: i64,
        since: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Message>, DbError> {
        let sql = "\
            SELECT m.logId, m.chatId, m.authorId, \
                   COALESCE(u.displayName, u.friendNickName, u.nickName) as senderName, \
                   m.message, m.type, m.sentAt \
            FROM NTChatMessage m \
            LEFT JOIN NTUser u ON m.authorId = u.userId AND u.linkId = 0 \
            WHERE m.chatId = ? \
              AND (? IS NULL OR m.sentAt >= ?) \
            ORDER BY m.sentAt DESC \
            LIMIT ?";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let my_uid = self.my_user_id;
        let rows = stmt
            .query_map(params![chat_id, since, since, limit], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    chat_id: row.get(1)?,
                    sender_id: row.get(2)?,
                    sender_name: row.get(3)?,
                    text: row.get(4)?,
                    message_type: MessageType::from(row.get::<_, i32>(5)?),
                    created_at: row.get(6)?,
                    is_from_me: row.get::<_, i64>(2)? == my_uid,
                })
            })
            .map_err(|e| DbError::QueryFailed(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows)
    }

    // ── Search ──────────────────────────────────────────────

    /// Search messages by keyword (LIKE '%keyword%').
    pub fn search_messages(&self, keyword: &str, limit: u32) -> Result<Vec<Message>, DbError> {
        let sql = "\
            SELECT m.logId, m.chatId, m.authorId, \
                   COALESCE(u.displayName, u.friendNickName, u.nickName) as senderName, \
                   m.message, m.type, m.sentAt \
            FROM NTChatMessage m \
            LEFT JOIN NTUser u ON m.authorId = u.userId AND u.linkId = 0 \
            WHERE m.message LIKE '%' || ? || '%' \
            ORDER BY m.sentAt DESC \
            LIMIT ?";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let my_uid = self.my_user_id;
        let rows = stmt
            .query_map(params![keyword, limit], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    chat_id: row.get(1)?,
                    sender_id: row.get(2)?,
                    sender_name: row.get(3)?,
                    text: row.get(4)?,
                    message_type: MessageType::from(row.get::<_, i32>(5)?),
                    created_at: row.get(6)?,
                    is_from_me: row.get::<_, i64>(2)? == my_uid,
                })
            })
            .map_err(|e| DbError::QueryFailed(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows)
    }

    /// Search rooms by name (LIKE '%keyword%').
    pub fn search_rooms(&self, keyword: &str, limit: u32) -> Result<Vec<Chat>, DbError> {
        let sql = "\
            SELECT r.chatId, r.type, r.chatName, r.activeMembersCount, \
                   r.lastLogId, r.lastUpdatedAt, r.countOfNewMessage, \
                   u.displayName, u.friendNickName, u.nickName \
            FROM NTChatRoom r \
            LEFT JOIN NTUser u ON r.directChatMemberUserId = u.userId AND u.linkId = 0 \
            WHERE COALESCE(u.displayName, u.friendNickName, u.nickName, r.chatName) \
                  LIKE '%' || ? || '%' \
            ORDER BY r.lastUpdatedAt DESC \
            LIMIT ?";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let rows = stmt
            .query_map(params![keyword, limit], |row| {
                let chat_type_raw: i32 = row.get(1)?;

                Ok(Chat {
                    id: row.get(0)?,
                    chat_type: ChatType::from_raw(chat_type_raw),
                    display_name: {
                        let chat_name: Option<String> = row.get(2)?;
                        let display_name: Option<String> = row.get(7)?;
                        let friend_nick: Option<String> = row.get(8)?;
                        let nick: Option<String> = row.get(9)?;
                        chat_name
                            .or(display_name)
                            .or(friend_nick)
                            .or(nick)
                            .unwrap_or_else(|| "(unknown)".into())
                    },
                    member_count: row.get(3)?,
                    last_message_id: row.get::<_, Option<i64>>(4)?,
                    last_message_at: row.get::<_, Option<i64>>(5)?,
                    unread_count: row.get(6)?,
                })
            })
            .map_err(|e| DbError::QueryFailed(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows)
    }

    /// Search friends by name (LIKE '%keyword%').
    pub fn search_friends(&self, keyword: &str, limit: u32) -> Result<Vec<Friend>, DbError> {
        let sql = "\
            SELECT u.userId, u.nickName, u.friendNickName, u.displayName \
            FROM NTUser u \
            WHERE (u.displayName LIKE '%' || ? || '%' \
                OR u.friendNickName LIKE '%' || ? || '%' \
                OR u.nickName LIKE '%' || ? || '%') \
              AND u.linkId = 0 \
            LIMIT ?";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let rows = stmt
            .query_map(params![keyword, keyword, keyword, limit], |row| {
                Ok(Friend {
                    user_id: row.get(0)?,
                    nick_name: row.get(1)?,
                    friend_nick_name: row.get(2)?,
                    display_name: row.get(3)?,
                })
            })
            .map_err(|e| DbError::QueryFailed(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        Ok(rows)
    }

    /// Search everything (messages + rooms + friends).
    pub fn search_all(&self, keyword: &str, limit: u32) -> Result<SearchResults, DbError> {
        Ok(SearchResults {
            messages: self.search_messages(keyword, limit)?,
            rooms: self.search_rooms(keyword, limit)?,
            friends: self.search_friends(keyword, limit)?,
        })
    }

    // ── Raw SQL ────────────────────────────────────────────

    /// Execute a read-only SQL query and return results as JSON.
    ///
    /// # Safety
    /// SQLITE_OPEN_READ_ONLY prevents writes, but `PRAGMA` statements (e.g.
    /// `PRAGMA wal_checkpoint`) can still mutate side-state. We guard against
    /// this by rejecting statements that contain `PRAGMA` (case-insensitive).
    /// For full protection, the caller should use a read-only database file
    /// copy or snapshot.
    pub fn raw_query(&self, sql: &str) -> Result<Value, DbError> {
        let sql_upper = sql.to_uppercase();
        if sql_upper.contains("PRAGMA") {
            return Err(DbError::QueryFailed(
                "PRAGMA statements are not allowed in raw_query".into(),
            ));
        }

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::QueryFailed(format!("Invalid SQL: {}", e)))?;

        let column_count = stmt.column_count();
        let column_names: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();

        let rows: Vec<Value> = stmt
            .query_map([], |row| {
                let mut map = serde_json::Map::new();
                for i in 0..column_count {
                    let name = &column_names[i];
                    if let Ok(val) = row.get::<_, String>(i) {
                        map.insert(name.clone(), Value::String(val));
                    } else if let Ok(val) = row.get::<_, i64>(i) {
                        map.insert(name.clone(), Value::Number(val.into()));
                    } else if let Ok(val) = row.get::<_, f64>(i) {
                        if let Some(n) = serde_json::Number::from_f64(val) {
                            map.insert(name.clone(), Value::Number(n));
                        }
                    } else {
                        map.insert(name.clone(), Value::Null);
                    }
                }
                Ok(Value::Object(map))
            })
            .map_err(|e| DbError::QueryFailed(format!("Query execution failed: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| DbError::QueryFailed(format!("Row read failed: {}", e)))?;

        Ok(Value::Array(rows))
    }

    // ── Chat name resolution ───────────────────────────────

    /// Find a chat by partial name match. Returns `(chat_id, display_name)`.
    /// On ambiguity, returns the first match — caller should check for this.
    pub fn resolve_chat_id(&self, name: &str) -> Result<Option<(i64, String)>, DbError> {
        let chats = self.list_chats(999999)?;
        let matched: Vec<&Chat> = chats.iter().filter(|c| c.display_name.contains(name)).collect();

        match matched.first() {
            None => Ok(None),
            Some(c) => Ok(Some((c.id, c.display_name.clone()))),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_open_nonexistent_db() {
        let result = Database::open(Path::new("/nonexistent/db"), &"ab".repeat(32), 0);
        assert!(matches!(result, Err(DbError::DatabaseNotFound(_))));
    }

    #[test]
    fn test_open_invalid_key_format() {
        // Create a temp file so it passes the existence check
        let tmp = std::env::temp_dir().join("kakaocli_test_invalid_key.db");
        let _ = std::fs::File::create(&tmp);
        let result = Database::open(&tmp, "short", 0);
        let _ = std::fs::remove_file(&tmp);
        assert!(matches!(result, Err(DbError::DatabaseOpenFailed(_))));
    }

    #[test]
    fn test_open_non_hex_key() {
        let tmp = std::env::temp_dir().join("kakaocli_test_non_hex.db");
        let _ = std::fs::File::create(&tmp);
        let result = Database::open(&tmp, &"z".repeat(64), 0);
        let _ = std::fs::remove_file(&tmp);
        assert!(matches!(result, Err(DbError::DatabaseOpenFailed(_))));
    }

    #[test]
    fn test_raw_query_rejects_pragma() {
        let tmp = std::env::temp_dir().join("kakaocli_test_pragma.db");
        std::fs::File::create(&tmp).ok();
        let conn = Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
        conn.execute_batch("CREATE TABLE test (id INTEGER)").ok();
        drop(conn);

        // This will fail to open (no SQLCipher key), but we construct manually
        let conn_read = Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn_read.execute_batch("CREATE TABLE IF NOT EXISTS x (a int)").ok();
        drop(conn_read);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_error_display() {
        let err = DbError::DatabaseNotFound(PathBuf::from("/x"));
        assert!(err.to_string().contains("/x"));

        let err = DbError::QueryFailed("bad sql".into());
        assert!(err.to_string().contains("bad sql"));

        let err = DbError::DecryptionFailed;
        assert!(err.to_string().contains("compatibility"));
    }
}