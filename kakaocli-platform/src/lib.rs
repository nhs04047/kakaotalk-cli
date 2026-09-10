//! kakaocli-platform: Platform-specific backend (macOS AX, Windows DEK + UIA).
//!
//! This crate compiles on all platforms but only runs on macOS and Windows.
//! Linux/build hosts get a stub that panics at runtime.

#[cfg(target_os = "macos")]
mod darwin;
#[cfg(target_os = "macos")]
pub use darwin::DarwinBackend as Platform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::WindowsBackend as Platform;
#[cfg(target_os = "windows")]
pub mod dek;
#[cfg(target_os = "windows")]
pub(crate) mod winsend;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod stub;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use stub::StubBackend as Platform;

use kakaocli_core::model::DbKey;
use serde::Serialize;
use thiserror::Error;

/// AX tree node for inspect command
#[derive(Debug, Clone, Serialize)]
pub struct AxNode {
    pub role: String,
    pub title: String,
    pub description: String,
    pub focused: bool,
    pub selected: bool,
    pub children: Vec<AxNode>,
}

impl AxNode {
    pub fn new(role: &str) -> Self {
        Self {
            role: role.to_string(),
            title: String::new(),
            description: String::new(),
            focused: false,
            selected: false,
            children: Vec::new(),
        }
    }
}

#[derive(Error, Debug)]
pub enum PlatformError {
    #[error("App not found or not running")]
    AppNotAvailable,
    #[error("Not logged in")]
    NotLoggedIn,
    #[error("DEK scan failed: {0}")]
    DekScanFailed(String),
    #[error("UI automation error: {0}")]
    UiError(String),
    #[error("Target chat verification failed: expected \"{expected}\", got \"{actual}\"")]
    ChatVerificationFailed { expected: String, actual: String },
    #[error("Multiple chats match \"{name}\": {matches:?} — use a more specific name")]
    AmbiguousChatName { name: String, matches: Vec<String> },
    #[error("{0}")]
    Other(String),
}

impl From<String> for PlatformError {
    fn from(s: String) -> Self {
        PlatformError::Other(s)
    }
}

impl From<&str> for PlatformError {
    fn from(s: &str) -> Self {
        PlatformError::Other(s.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppStatus {
    Ready,
    LoggedOut,
    NotRunning,
    DbAccessible,
}

pub trait PlatformBackend {
    /// DB 키 획득 (macOS: KDF 계산 + userId 자동탐색/오버라이드, Windows: DEK 프로세스 스캔)
    fn resolve_db_key(user_id: Option<u64>) -> Result<DbKey, PlatformError>;

    /// 앱 상태 확인
    fn check_status() -> Result<AppStatus, PlatformError>;

    /// 로그인 (크레덴셜을 OS 키체인에 저장)
    fn login(email: &str, password: &str) -> Result<(), PlatformError>;

    /// 메시지 전송
    /// - chat_name으로 채팅방을 찾아 열고 text를 입력 후 전송
    /// - 전송 전 열린 방의 제목을 재확인 (ChatVerificationFailed)
    /// - 동일한 chat_name이 여러 개면 AmbiguousChatName
    fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError>;

    /// AX 트리 덤프 (디버깅용, macOS only)
    fn dump_ax_tree(chat: Option<&str>, max_depth: u32) -> Result<AxNode, PlatformError>;
}