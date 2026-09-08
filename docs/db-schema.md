# kakaocli-db: SQLCipher Database Reader

## 개요

kakaocli-db는 macOS/Windows KakaoTalk의 SQLCipher 암호화 DB를 읽는 크레이트.

## DB 스키마 (macOS 기준, kakaocli에서 확인)

### 주요 테이블

```sql
-- 채팅방
NTChatRoom (
    chatId INTEGER PRIMARY KEY,      -- 채팅방 ID
    type INTEGER,                     -- 0=직챗, 1=단톡, 5=나와의채팅
    chatName TEXT,                    -- 그룹명 (보통 NULL)
    activeMembersCount INTEGER,
    lastLogId INTEGER,
    lastUpdatedAt INTEGER,            -- Unix timestamp (초)
    countOfNewMessage INTEGER,
    directChatMemberUserId INTEGER,   -- 직챗 상대방 userId
    displayMemberIds BLOB,            -- 단톡: binary plist of [NSNumber]
    linkId INTEGER,                   -- 오픈채팅 링크 ID
    lastMessage TEXT
);

-- 메시지
NTChatMessage (
    logId INTEGER PRIMARY KEY,        -- 메시지 ID (단조 증가)
    chatId INTEGER,                   -- NTChatRoom.chatId
    authorId INTEGER,                 -- 보낸 사람 userId
    message TEXT,                     -- 메시지 본문
    type INTEGER,                     -- 메시지 타입 (1=텍스트)
    sentAt INTEGER,                   -- Unix timestamp (초)
    -- 기타: attachment, supplement, referer 등
);

-- 사용자
NTUser (
    userId INTEGER PRIMARY KEY,
    nickName TEXT,                    -- 닉네임
    friendNickName TEXT,              -- 친구 닉네임 (내가 지정)
    displayName TEXT,                 -- 표시 이름
    linkId INTEGER DEFAULT 0,         -- 0=내 친구, >0=오픈채팅
    profileImageURL TEXT
);

-- 내 사용자 ID
NTChatContext (count INTEGER);        -- 단일 행, 내 userId 포함

-- 오픈채팅 링크
NTOpenLink (
    linkId INTEGER PRIMARY KEY,
    linkName TEXT                     -- 오픈채팅방 이름
);
```

## 쿼리 상세

### list_chats

```sql
SELECT r.chatId, r.type, r.chatName, r.activeMembersCount,
       r.lastLogId, r.lastUpdatedAt, r.countOfNewMessage,
       u.displayName, u.friendNickName, u.nickName
FROM NTChatRoom r
LEFT JOIN NTUser u ON r.directChatMemberUserId = u.userId AND u.linkId = 0
ORDER BY r.lastUpdatedAt DESC
LIMIT ?
```

**이름 결정 로직:**
1. `NTChatRoom.chatName`이 있으면 사용 (사용자 지정 그룹명)
2. 직챗: `NTUser.displayName` → `friendNickName` → `nickName`
3. 단톡: `displayMemberIds` bplist 파싱 → 각 userId → NTUser에서 이름 조회
4. 오픈채팅: `NTOpenLink.linkName`
5. 나와의채팅 (type=5): "Self-chat (Notes)"
6. 모두 실패: "(unknown)"

### get_messages

```sql
SELECT m.logId, m.chatId, m.authorId,
       COALESCE(u.displayName, u.friendNickName, u.nickName) as senderName,
       m.message, m.type, m.sentAt
FROM NTChatMessage m
LEFT JOIN NTUser u ON m.authorId = u.userId AND u.linkId = 0
WHERE m.chatId = ?
  AND (? IS NULL OR m.sentAt >= ?)
ORDER BY m.sentAt DESC
LIMIT ?
```

### search_messages

```sql
SELECT m.logId, m.chatId, m.authorId,
       COALESCE(u.displayName, u.friendNickName, u.nickName) as senderName,
       m.message, m.type, m.sentAt
FROM NTChatMessage m
LEFT JOIN NTUser u ON m.authorId = u.userId AND u.linkId = 0
WHERE m.message LIKE '%' || ? || '%'
ORDER BY m.sentAt DESC
LIMIT ?
```

### search_friends (친구 이름 검색)

```sql
SELECT u.userId, u.nickName, u.friendNickName, u.displayName
FROM NTUser u
WHERE (u.displayName LIKE '%' || ? || '%'
    OR u.friendNickName LIKE '%' || ? || '%'
    OR u.nickName LIKE '%' || ? || '%')
  AND u.linkId = 0  -- 내 친구만 (오픈채팅 제외)
LIMIT ?
```

### search_rooms (채팅방 이름 검색)

```sql
-- 직챗: 친구 이름 기준
SELECT r.chatId, r.type,
       COALESCE(u.displayName, u.friendNickName, u.nickName) as name,
       r.activeMembersCount, r.lastUpdatedAt
FROM NTChatRoom r
LEFT JOIN NTUser u ON r.directChatMemberUserId = u.userId AND u.linkId = 0
WHERE COALESCE(u.displayName, u.friendNickName, u.nickName, r.chatName) LIKE '%' || ? || '%'
ORDER BY r.lastUpdatedAt DESC
LIMIT ?
```

### search_all (통합 검색)

```sql
-- 메시지 + 친구 + 채팅방 결과를 각각 LIMIT까지 가져와서 병합
-- 결과에 type 필드 추가: "message" / "friend" / "room"
```

## SQLCipher 연결 (Opus 검증 반영)

```rust
fn open_sqlcipher(path: &Path, key_hex: &str) -> Result<Connection> {
    // key_hex: 64-char hex string (32 bytes raw key)
    // 각 compat 모드는 **새 커넥션**에서 시도 (재사용 불가)

    for compat in [3, 4] {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.execute_batch(&format!("PRAGMA cipher_compatibility = {}", compat))?;
        // Use x'<hex>' for raw key — NOT '<hex>' which would re-KDF
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\"", key_hex))?;
        if conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .is_ok()
        {
            return Ok(conn);
        }
        // Compat mode failed → drop conn and try next
    }

    Err(Error::DecryptionFailed(
        "All cipher compatibility modes (3, 4) failed with the derived key".into()
    ))
}
```

## Windows DB

Windows `.edb` 파일은 동일한 SQLCipher 포맷이지만:
- 파일 확장자가 `.edb` (`.db` 아님)
- 키는 프로세스 메모리에서 DEK(32바이트 hex)로 획득
- 동일한 쿼리 사용 가능 (획득한 키로 동일하게 open)

```rust
// Windows에서 DEK 획득 후:
let dek = WindowsBackend::resolve_db_key()?;  // { dek: [u8; 32], db_path: PathBuf }
let conn = open_sqlcipher(&dek.db_path, &hex::encode(dek.dek))?;
// 이후 동일한 쿼리 실행
```