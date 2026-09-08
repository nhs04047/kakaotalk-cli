# kakaocli-rs: Cross-Platform KakaoTalk CLI

**Rust로 만드는 macOS + Windows 카카오톡 CLI**

> kakaocli (Swift)의 방식을 계승하되, macOS만 지원하던 한계를 넘어 Windows까지 지원

---

## 프로젝트 개요

| 항목 | 내용 |
|------|------|
| 프로젝트명 | kakaocli-rs |
| 언어 | Rust (edition 2021) |
| 타겟 | macOS 14+, Windows 10/11 x64 |
| 라이선스 | MIT |
| 저장소 | https://github.com/nhs04047/kakaotalk-cli |
| 설계 저장소 | https://github.com/nhs04047/kakaotalk-cli-design |

## 핵심 목표

1. macOS 카카오톡 DB 읽기/전송 (kakaocli 방식 계승)
2. Windows 카카오톡 DB 읽기/전송 (DEK 스캔 + UIAutomation)
3. 단일 CLI 바이너리 (cargo build → 하나의 실행파일)

## 비목표 (v1에서 제외)

- AI 에이전트 연동 (MCP, webhook)
- Linux 지원
- 미디어 메시지 처리
- Harvest / Vision OCR

---

## 아키텍처 개요

```
┌─────────────────────────────────────────────────────┐
│                 kakaocli-rs CLI                     │
│                  (clap + serde)                     │
├──────────┬──────────┬──────────┬──────────┬─────────┤
│  core    │    db    │ platform │   auth   │   cli   │
│ (model)  │ (reader) │ (cfg fn) │ (keytar) │(clap)   │
│ (kdf)    │ (sqlite) │ (да/win) │          │         │
│ (path)   │          │          │          │         │
└──────────┴──────────┴──────────┴──────────┴─────────┘
         │           │           │
         ▼           ▼           ▼
    ┌─────────┐ ┌─────────┐ ┌─────────┐
    │ macOS   │ │ SQLCipher│ │ Windows │
    │AXUIElem │ │   DB    │ │  DEK    │
    │         │ │         │ │UIAutom. │
    └─────────┘ └─────────┘ └─────────┘
```

## 5개 크레이트

| 크레이트 | 의존성 | 역할 | 플랫폼 |
|----------|--------|------|--------|
| `kakaocli-core` | ring, plist, chrono | 모델, 키 유도, DB 경로, plist 파싱 | 공통 |
| `kakaocli-db` | rusqlite + sqlcipher | SQLCipher 읽기/쿼리 | 공통 (macOS/Windows) |
| `kakaocli-platform` | objc2 or windows-rs | 플랫폼 특화 (전송, DEK) | cfg 분기 |
| `kakaocli-auth` | keyring | 인증 정보 저장 | 공통 |
| `kakaocli-cli` | clap + serde_json | CLI 명령어 | 공통 |

---

## 플랫폼별 동작 방식

### macOS: 읽기

```
KakaoTalk.app
  │ plist: FSChatWindowTransparency → 사용자 ID
  │ ioreg → device UUID
  ▼
PBKDF2-SHA256 (100,000회, blluv 알고리즘)
  │ password = 복합 문자열 (사용자ID + UUID + 고정값)
  │ salt = UUID 기반
  ▼
32바이트 raw key (PBKDF2 출력 중 앞 32바이트)
  │ hex 인코딩 → "ab12cd34..."
  ▼
WAL 처리: 라이브 DB는 temp 파일로 복사 후 오픈
  ▼
rusqlite + bundled-sqlcipher
  │ compat mode별로 **새 커넥션** (재사용 안 됨)
  │ PRAGMA cipher_compatibility = 3  (cipher_default_compatibility 아님!)
  │ PRAGMA key = "x'<hex>'..."       (따옴표x → 원시키! passphrase 아님!)
  │   └── 주의: PRAGMA key = '...' 는 passphrase로 재-KDF되어 실패
  │          PRAGMA key = "x'..."    는 raw key를 직접 사용
  │   실패 → PRAGMA cipher_compatibility = 4로 재시도 (새 커넥션)
  │   성공 → SELECT ... FROM NTChatRoom, NTChatMessage, NTUser
  ▼
JSON 출력
```

**WAL 처리:** KakaoTalk이 라이브로 사용 중인 DB는 WAL/WAL2 저널 모드일 수 있음.  
→ SQLITE_OPEN_READONLY로 열면 읽기 가능하나, WAL 체크포인트가 꼬일 위험이 있음.  
→ **temp 디렉토리에 복사본을 만들고 읽기 전용으로 오픈**하는 것이 안전.  
→ macOS Phase 1 MVP에서는 SQLITE_OPEN_READONLY로 진행 (복사는 추후).

### macOS: 전송

```
kakaocli send "지수" "안녕!"
  │
  ▼
macOS Accessibility (AXUIElement) + CoreGraphics (CGEvent)
  │ FFI crate: accessibility + core-foundation + core-graphics + objc2
  │
  ├── 1. NSWorkspace → KakaoTalk 실행/활성화 (objc2)
  │     로그인 필요시 자동 로그인 (keychain → 크리덴셜)
  │
  ├── 2. 메인 윈도우 찾기 (AXUIElementCopyAttributeValue)
  │     id="Main Window" (kAXWindowsAttribute가 AXApplication 반환 가능)
  │
  ├── 3. 채팅방 리스트에서 "지수" 검색
  │     AX 셀렉션 → kAXSelectedRowsAttribute + Enter(CGEvent)
  │     ⚠️ double-click 사용 금지: 오프스크린 좌표 문제
  │
  ├── 4. **대상 방 검증** ← 안전성
  │     열린 창의 제목(NSCreateObjectWithBytes)을 읽어
  │     실제로 "지수" 채팅방이 열렸는지 확인
  │     불일치 시 에러 반환 (전송 차단)
  │
  ├── 5. AXTextField에 타이핑
  │     CGEventKeyboard로 유니코드 입력 (한글 지원)
  │
  └── 6. Enter → 전송 (CGEvent)
```

**전송 안전 정책:**
1. `--me` 플래그: 나와의 채팅으로 전송 (다른 사람 채팅에 실수로 전송 방지)
2. `--dry-run`: 실제 전송 없이 대상 채팅방까지만 열고 닫음
3. **대상 방 확정 검증:** 채팅방을 연 후 창 제목을 읽어 의도한 방이 맞는지 확인
4. **이름 모호성:** 같은 이름의 채팅방이 여러 개면 첫 번째 선택 후 경고 출력
5. **Rate limit:** 전송 간 2초 이상 간격

### Windows: 읽기

```
KakaoTalk.exe (실행 중)
  │
  ▼
windows-rs → 프로세스 메모리 스캔
  1. CreateToolhelp32Snapshot → KakaoTalk.exe PID 찾기
  2. OpenProcess(PROCESS_VM_READ)
  3. 힙 영역에서 32바이트 DEK 후보 스캔
  4. 각 후보를 BCrypt AES-256으로 page-1 복호화 검증
  ▼
검증된 DEK (32바이트)
  │
  ▼
rusqlite + bundled-sqlcipher
  │ open with key = DEK hex string
  │ SELECT ... (동일한 쿼리)
  ▼
JSON 출력
```

### Windows: 전송

```
kakaocli send "지수" "안녕!"
  │
  ▼
windows-rs → FindWindow → KakaoTalk 윈도우 찾기
  ▼
windows-rs → UIAutomation
  1. Condition: 이름="지수" ListItem 찾기
  2. InvokePattern → 채팅방 열기
  3. Edit 컨트롤 찾기 → SetFocus → SendKeys
  4. 전송 버튼 Invoke
```

---

## CLI 명령어

```bash
kakaocli check|status       # 앱/권한/DB 상태 확인
kakaocli auth               # DB 복호화 검증
kakaocli chats|rooms [--limit N] [--json]          # 채팅방 목록
kakaocli msg|messages --chat "이름" [--since 1h] [--limit N] [--json]
kakaocli find|search "키워드" [--json]              # 메시지 내용 LIKE (% 자동)
kakaocli find|search "패턴" --exact [--json]        # 정확히 일치 (=)
kakaocli find|search "패턴" --regex [--json]        # 정규식 (REGEXP)
kakaocli find "이름" --rooms [--json]               # 채팅방 이름 LIKE
kakaocli find "이름" --friends [--json]              # 친구 이름 LIKE
kakaocli find "키워드" --all [--json]                # 메시지+채팅방+친구 통합
kakaocli query "SELECT ..."  # raw SQL (읽기 전용)
kakaocli send|say "이름" "메시지" [--dry-run]
kakaocli send|say --me _ "메시지"
kakaocli sync|tail [--follow] [--interval 2]
kakaocli inspect [--chat "이름"]  # AX/UIA 트리 덤프 (디버깅)
kakaocli login [--email ... --password ...]
kakaocli login --status
kakaocli login --clear
```

---

## 크레이트 상세

### kakaocli-core

```rust
// 모델 (serde + json 출력)
pub struct Chat { id: i64, chat_type: ChatType, display_name: String, member_count: i32, unread_count: i32, last_message_at: Option<i64> }
pub struct Message { id: i64, chat_id: i64, sender_id: i64, sender_name: Option<String>, text: Option<String>, message_type: MessageType, created_at: i64, is_from_me: bool }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    Text = 1,     // ❌ Rust enum variant can't have raw values like this
    // → Use #[repr(i32)] + manual From<i32> impl instead
}
// Opus: 위 enum은 컴파일 안 됨. 아래 방식으로 수정:
// #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
// #[repr(i32)]
// pub enum MessageType { Text = 1, Photo = 2, Video = 3, Unknown(i32) }
// impl From<i32> for MessageType { ... }

// 키 유도 (PBKDF2-SHA256, ring crate 사용)
/// KDF 출력: 128바이트 → 그중 첫 32바이트가 raw key
/// raw key를 hex 인코딩 → PRAGMA key = "x'<hex>'" 형태로 사용
pub fn derive_mac_key(user_id: u64, device_uuid: &str) -> String;
pub fn derive_mac_db_name(user_id: u64, device_uuid: &str) -> String;

// DB 경로
pub fn mac_container_path() -> PathBuf;
pub fn mac_db_files() -> Vec<PathBuf>;
pub fn windows_chat_data_path() -> PathBuf;
pub fn windows_edb_files() -> Vec<PathBuf>;

// 사용자 ID 탐지
/// macOS: FSChatWindowTransparency plist에서 사용자 ID 추출
/// plist crate로 binary plist 파싱
pub fn mac_user_id() -> Result<u64>;

/// macOS: IOPlatformUUID (ioreg)
pub fn mac_platform_uuid() -> Result<String>;

/// TCC 권한 확인 (macOS)
pub fn check_full_disk_access() -> bool;
pub fn check_accessibility_permission() -> bool;
```

### kakaocli-db

```rust
pub struct Database { /* rusqlite::Connection + cached my_user_id */ }

impl Database {
    /// SQLCipher 오픈: compat 모드 3, 4 각각 **새 커넥션**으로 시도
    /// key는 32바이트 hex 문자열 ("ab12cd...")
    /// PRAGMA key = "x'<hex>'"  (원시키, 재-KDF 없음)
    /// PRAGMA cipher_compatibility = N  (NOT cipher_default_compatibility)
    pub fn open_with_key(path: &Path, key_hex: &str) -> Result<Self>;
    pub fn list_chats(&self, limit: u32) -> Result<Vec<Chat>>;
    pub fn get_messages(&self, chat_id: i64, since: Option<NaiveDateTime>, limit: u32) -> Result<Vec<Message>>;
    pub fn search_messages(&self, keyword: &str, limit: u32) -> Result<Vec<Message>>;
    pub fn raw_query(&self, sql: &str) -> Result<Value>;  // serde_json::Value
    pub fn resolve_chat_name(&self, chat_id: i64) -> Result<Option<String>>;
}
```

### kakaocli-platform

```rust
#[cfg(target_os = "macos")]
pub use darwin::DarwinBackend as Platform;

#[cfg(target_os = "windows")]
pub use windows::WindowsBackend as Platform;

pub trait PlatformBackend {
    /// DB 키 획득
    fn resolve_db_key() -> Result<DbKey, PlatformError>;

    /// 앱 상태 확인
    fn check_status() -> Result<AppStatus, PlatformError>;

    /// 로그인 (크레덴셜 저장)
    fn login(email: &str, password: &str) -> Result<(), PlatformError>;

    /// 메시지 전송
    fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError>;
}

pub enum AppStatus {
    Ready,          // 앱 켜져 있고 로그인됨
    LoggedOut,      // 앱은 켜져 있으나 로그아웃 상태
    NotRunning,     // 앱 미실행
    DbAccessible,   // 앱 없지만 DB는 읽기 가능 (macOS only)
}
```

### kakaocli-auth

```rust
pub fn store_credentials(email: &str, password: &str) -> Result<()>;
pub fn get_credentials() -> Result<(String, String)>;
pub fn clear_credentials() -> Result<()>;
pub fn has_credentials() -> bool;
// macOS: Keychain (service="com.kakaocli-rs.credentials")
// Windows: Credential Manager
```

### kakaocli-cli

```rust
// clap subcommands
#[derive(Parser)]
#[command(name="kakaocli")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 앱/권한/DB 상태 확인
    #[command(aliases = ["check"])]
    Status,

    /// DB 복호화 검증
    Auth { verbose: bool, user_id: Option<u64>, uuid: Option<String>, force: bool },

    /// 채팅방 목록
    #[command(aliases = ["rooms"])]
    Chats { limit: u32, json: bool },

    /// 메시지 조회
    #[command(aliases = ["messages"])]
    Msg { chat: String, since: Option<String>, limit: u32, json: bool },

    /// 메시지 검색 (LIKE '%keyword%')
    /// --exact: 정확히 일치
    /// --regex: 정규식 검색
    /// --rooms: 채팅방 이름 검색
    /// --friends: 친구 이름 검색
    /// --all: 메시지+채팅방+친구 통합 검색
    #[command(aliases = ["find"])]
    Search { keyword: String, json: bool, exact: bool, regex: bool, rooms: bool, friends: bool, all: bool },

    /// Raw SQL 쿼리 (읽기 전용)
    Query { sql: String },

    /// 메시지 전송
    #[command(aliases = ["say"])]
    Send { chat: String, message: String, me: bool, dry_run: bool },

    /// 새 메시지 모니터링
    #[command(aliases = ["tail"])]
    Sync { follow: bool, interval: u32 },

    /// AX/UIA 트리 덤프
    Inspect { chat: Option<String> },

    /// 로그인/인증
    Login { email: Option<String>, password: Option<String>, status: bool, clear: bool },
}
```

---

## 빌드 전략

### Cargo.toml (workspace root)

```toml
[workspace]
members = ["kakaocli-core", "kakaocli-db", "kakaocli-platform", "kakaocli-auth", "kakaocli-cli"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
```

### Platform conditional compilation

```rust
// kakaocli-platform/Cargo.toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-foundation = "0.3"
objc2-app-kit = "0.3"
accessibility = "0.2"              # AXUIElement Rust wrapper
core-foundation = "0.10"           # CFString 등
core-graphics = "0.24"             # CGEvent (키보드 입력, 클릭)

[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.60", features = [
    "Win32_System_Diagnostics_ToolHelp",   # CreateToolhelp32Snapshot
    "Win32_System_Threading",              # OpenProcess, ReadProcessMemory
    "Win32_System_Memory",                 # VirtualQueryEx
    "Win32_Security",                      # BCrypt (DEK 검증)
    "Win32_UI_Accessibility",              # UIAutomation
    "Win32_UI_WindowsAndMessaging",        # FindWindow, SendKeys
    "Win32_UI_Input_KeyboardAndMouse",     # SendInput
    "Win32_Foundation",                    # HWND, HANDLE 등 기본 타입
    "Win32_System_LibraryLoader",          # GetModuleHandle
    "Win32_Security_Cryptography",         # BCrypt AES-256
    "Win32_System_Com",                    # CoCreateInstance (UIA 초기화)
]}

[target.'cfg(not(any(target_os = "macos", target_os = "windows")))'.dependencies]
# Linux: stub only (compile check, panic at runtime)
```

### kakaocli-db

```toml
[dependencies]
rusqlite = { version = "0.40", features = ["bundled-sqlcipher-vendored-openssl"] }
# macOS: Security.framework 사용 (OpenSSL 불필요)
# Windows: vendored OpenSSL 정적 링킹
```

---

## 단계별 로드맵

### Phase 1: macOS 읽기 MVP
- `kakaocli-core`: 모델 + KDF + 경로
- `kakaocli-db`: SQLCipher 오픈 → chats/messages/search/query
- `kakaocli-cli`: status/auth/chats/messages/search/query
- **검증:** macOS에서 실제 채팅 읽기 성공

### Phase 2: macOS 전송
- `kakaocli-platform/darwin.rs`: objc2 AXUIElement 전송
- `kakaocli-auth`: 키체인
- CLI: send/login/inspect
- **검증:** macOS에서 실제 전송 성공

### Phase 3: Windows MVP
- `kakaocli-platform/windows.rs`: DEK 스캐너
- `kakaocli-db`: DEK → DB 연결 통합
- Windows UIAutomation 전송
- **검증:** Windows에서 읽기 + 전송 성공

### Phase 4: 고급 기능
- sync (폴링/NDJSON)
- harvest (이름 수집)

---

## 참여 설계 원칙

1. **kakaocli(Swift)의 검증된 방식을 최대한 계승** — macOS KDF/DB 쿼리는 그대로 이식
2. **플랫폼 특화 코드는 trait으로 추상화** — 비즈니스 로직과 분리
3. **Windows DEK 스캔은 MoniKa의 접근법 참고** — 프로세스 메모리 + BCrypt oracle
4. **원칙 #4 제외 (검증됨):** macOS는 DB만 있으면 읽기 명령어 작동. **Windows는 KakaoTalk.exe 실행 중이어야 함.**
5. **JSON 출력 기본** — AI 에이전트 연동 대비

---

## 참고 자료

- kakaocli 원본: https://github.com/silver-flight-group/kakaocli
- blluv's DB 복호화 gist: https://gist.github.com/blluv/8418e3ef4f4aa86004657ea524f2de14
- MoniKa (Windows DEK): https://github.com/maxswjeon/MoniKa
- rusqlite sqlcipher: https://crates.io/crates/rusqlite
- wacli (WhatsApp CLI, 영감): https://github.com/steipete/wacli