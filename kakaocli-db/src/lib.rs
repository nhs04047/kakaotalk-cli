//! kakaocli-db: SQLCipher database reader for KakaoTalk.
//!
//! Reads KakaoTalk's SQLCipher-encrypted local database.
//! All queries are read-only (SQLITE_OPEN_READ_ONLY).
//!
//! # Key format
//! Key must be a 256-char hex string (128 bytes full PBKDF2 output).
//! Passed as a SQLCipher passphrase via: `PRAGMA key = '<256-char-hex>'`
//! (kakaocli Swift uses `PRAGMA KEY='<hex>'` — SQLCipher re-derives the key).

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
    /// Uses the key as a passphrase via `PRAGMA key = '<hex>'` (kakaocli Swift compatible).
    pub fn open(path: &Path, key_hex: &str, my_user_id: i64) -> Result<Self, DbError> {
        if !path.exists() {
            return Err(DbError::DatabaseNotFound(path.to_path_buf()));
        }

        // Validate key format
        if key_hex.len() != 256 || !key_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DbError::DatabaseOpenFailed(
                "Key must be 256 hex characters (128 bytes PBKDF2 output)".into(),
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

            // Set key FIRST, then cipher_compatibility — SQLCipher derives the key
            // using the compatibility mode active at key-set time. If compat is
            // set before key, SQLCipher uses its own default (v4: 256000/SHA512)
            // instead of KakaoTalk's v3 (64000/SHA1) → "file is not a database".
            let pragma_sql = format!(
                "PRAGMA key = '{}'; PRAGMA cipher_compatibility = {};",
                key_hex, compat
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

    // ── Sync (증분 조회) ─────────────────────────────────────

    /// Fetch messages with `logId > after_log_id`, oldest first, for incremental
    /// streaming (`kakaocli sync`). `chat_id = None` scans all rooms.
    ///
    /// Ordered by `logId ASC` (unlike `get_messages`, which is newest-first) so a
    /// stream emits messages in arrival order. `limit` caps one poll's batch.
    pub fn messages_after(
        &self,
        after_log_id: i64,
        chat_id: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Message>, DbError> {
        let sql = "\
            SELECT m.logId, m.chatId, m.authorId, \
                   COALESCE(u.displayName, u.friendNickName, u.nickName) as senderName, \
                   m.message, m.type, m.sentAt \
            FROM NTChatMessage m \
            LEFT JOIN NTUser u ON m.authorId = u.userId AND u.linkId = 0 \
            WHERE m.logId > ? \
              AND (? IS NULL OR m.chatId = ?) \
            ORDER BY m.logId ASC \
            LIMIT ?";

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;

        let my_uid = self.my_user_id;
        let rows = stmt
            .query_map(params![after_log_id, chat_id, chat_id, limit], |row| {
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

    /// Current maximum `logId` (baseline for a fresh sync). `chat_id = None`
    /// spans all rooms. Returns 0 when there are no messages.
    pub fn max_log_id(&self, chat_id: Option<i64>) -> Result<i64, DbError> {
        let sql = "SELECT COALESCE(MAX(logId), 0) FROM NTChatMessage \
                   WHERE (? IS NULL OR chatId = ?)";
        self.conn
            .query_row(sql, params![chat_id, chat_id], |row| row.get(0))
            .map_err(|e| DbError::QueryFailed(e.to_string()))
    }

    /// Baseline `logId` for a `--since` backfill: `(min logId with sentAt >= since) - 1`,
    /// so the first message at/after `since_ts` is included. Returns `None` when no
    /// message is that recent (caller then falls back to the current MAX).
    pub fn log_id_before_since(
        &self,
        chat_id: Option<i64>,
        since_ts: i64,
    ) -> Result<Option<i64>, DbError> {
        let sql = "SELECT MIN(logId) FROM NTChatMessage \
                   WHERE sentAt >= ? AND (? IS NULL OR chatId = ?)";
        let min: Option<i64> = self
            .conn
            .query_row(sql, params![since_ts, chat_id, chat_id], |row| row.get(0))
            .map_err(|e| DbError::QueryFailed(e.to_string()))?;
        Ok(min.map(|m| m - 1))
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
        let result = Database::open(&tmp, &"z".repeat(256), 0);
        let _ = std::fs::remove_file(&tmp);
        assert!(matches!(result, Err(DbError::DatabaseOpenFailed(_))));
    }

    #[test]
    fn test_pragma_order_key_before_compat() {
        // PRAGMA 순서: key를 반드시 compat보다 먼저.
        // SQLCipher는 key 설정 시점의 compat로 키를 파생하므로,
        // compat를 먼저 주면 v4 기본값(256000/SHA512)이 적용되어
        // 카카오톡 v3 DB(64000/SHA1)를 열 수 없다.
        let key_hex = "a".repeat(256);
        let compat = 3;
        let pragma_sql = format!(
            "PRAGMA key = '{}'; PRAGMA cipher_compatibility = {};",
            key_hex, compat
        );
        // key가 앞에 오고 compat가 뒤에 온다
        let key_pos = pragma_sql.find("PRAGMA key").unwrap();
        let compat_pos = pragma_sql.find("PRAGMA cipher_compatibility").unwrap();
        assert!(key_pos < compat_pos, "key must precede cipher_compatibility");
        // 명시적 파라미터가 포함된다
        assert!(pragma_sql.contains(&format!("key = '{}'", key_hex)));
        assert!(pragma_sql.contains("cipher_compatibility = 3"));
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

    // ── Sync query tests (plaintext in-memory DB) ───────────

    /// Build an in-memory `Database` with the minimal NTChatMessage/NTUser
    /// schema. Rows: (logId, chatId, authorId, message, type, sentAt).
    fn sync_test_db(rows: &[(i64, i64, i64, &str, i32, i64)], my_uid: i64) -> Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE NTChatMessage (logId INTEGER, chatId INTEGER, authorId INTEGER, \
                 message TEXT, type INTEGER, sentAt INTEGER); \
             CREATE TABLE NTUser (userId INTEGER, linkId INTEGER, displayName TEXT, \
                 friendNickName TEXT, nickName TEXT);",
        )
        .unwrap();
        for (log_id, chat_id, author_id, msg, ty, sent_at) in rows {
            conn.execute(
                "INSERT INTO NTChatMessage (logId, chatId, authorId, message, type, sentAt) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![log_id, chat_id, author_id, msg, ty, sent_at],
            )
            .unwrap();
        }
        Database { conn, my_user_id: my_uid }
    }

    #[test]
    fn test_messages_after_ordering_and_boundary() {
        // logIds out of order in storage; query must return ASC and exclude == after.
        let db = sync_test_db(
            &[
                (3, 1, 100, "third", 1, 300),
                (1, 1, 100, "first", 1, 100),
                (2, 1, 200, "second", 1, 200),
            ],
            100,
        );

        let msgs = db.messages_after(1, None, 100).unwrap();
        // logId > 1 → excludes logId 1; ASC → [2, 3]
        assert_eq!(msgs.iter().map(|m| m.id).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(msgs[0].text.as_deref(), Some("second"));
    }

    #[test]
    fn test_messages_after_chat_filter_and_limit() {
        let db = sync_test_db(
            &[
                (1, 10, 1, "a", 1, 1),
                (2, 20, 1, "b", 1, 2),
                (3, 10, 1, "c", 1, 3),
                (4, 10, 1, "d", 1, 4),
            ],
            1,
        );

        // Only chat 10, logId > 0 → [1, 3, 4]; limit 2 → [1, 3]
        let msgs = db.messages_after(0, Some(10), 2).unwrap();
        assert_eq!(msgs.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 3]);

        // All rooms, logId > 2 → [3, 4]
        let all = db.messages_after(2, None, 100).unwrap();
        assert_eq!(all.iter().map(|m| m.id).collect::<Vec<_>>(), vec![3, 4]);
    }

    #[test]
    fn test_messages_after_is_from_me() {
        let db = sync_test_db(&[(1, 1, 100, "mine", 1, 1), (2, 1, 999, "theirs", 1, 2)], 100);
        let msgs = db.messages_after(0, None, 100).unwrap();
        assert!(msgs[0].is_from_me);
        assert!(!msgs[1].is_from_me);
    }

    #[test]
    fn test_max_log_id() {
        let empty = sync_test_db(&[], 1);
        assert_eq!(empty.max_log_id(None).unwrap(), 0);

        let db = sync_test_db(
            &[(5, 10, 1, "a", 1, 1), (9, 20, 1, "b", 1, 2), (7, 10, 1, "c", 1, 3)],
            1,
        );
        assert_eq!(db.max_log_id(None).unwrap(), 9);
        assert_eq!(db.max_log_id(Some(10)).unwrap(), 7);
        assert_eq!(db.max_log_id(Some(999)).unwrap(), 0);
    }

    #[test]
    fn test_log_id_before_since() {
        let db = sync_test_db(
            &[
                (1, 10, 1, "old", 1, 100),
                (2, 10, 1, "mid", 1, 200),
                (3, 10, 1, "new", 1, 300),
            ],
            1,
        );
        // since=200 → first msg at/after is logId 2 → baseline 1 (so logId 2 included)
        assert_eq!(db.log_id_before_since(None, 200).unwrap(), Some(1));
        // since=50 → first is logId 1 → baseline 0
        assert_eq!(db.log_id_before_since(None, 50).unwrap(), Some(0));
        // since=999 → nothing that recent → None
        assert_eq!(db.log_id_before_since(None, 999).unwrap(), None);
    }
}