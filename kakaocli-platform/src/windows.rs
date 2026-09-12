// Windows backend: DEK process memory scan + UIAutomation
// References:
// - MoniKa (https://github.com/maxswjeon/MoniKa)
// - windows-rs crate docs

use crate::*;
use kakaocli_core::model::DbKey;

pub struct WindowsBackend;

impl PlatformBackend for WindowsBackend {
    fn resolve_db_key(_user_id: Option<u64>) -> Result<DbKey, PlatformError> {
        // Windows uses **per-file DEKs** captured from process memory (see
        // `crate::dek`), not a single derived key + path. Read commands go
        // through `dek::open_edb`; this single-key entry point does not apply.
        Err(PlatformError::Other(
            "이 명령은 아직 Windows에서 지원되지 않습니다 (Windows는 파일별 DEK 사용 — chats/msg/find/query/send/auth/check 참고).".into(),
        ))
    }

    fn check_status() -> Result<AppStatus, PlatformError> {
        // On Windows the DEK lives only in the running process's memory, so
        // reading requires KakaoTalk to be running. Not running → can't decrypt.
        if crate::dek::find_kakao_pid().is_some() {
            Ok(AppStatus::Ready)
        } else {
            Ok(AppStatus::NotRunning)
        }
    }

    fn login(email: &str, password: &str) -> Result<(), PlatformError> {
        kakaocli_auth::store_credentials(email, password)
            .map_err(|e| PlatformError::Other(e.to_string()))
    }

    fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError> {
        crate::winsend::send_message(chat_name, text)
    }

    fn dump_ax_tree(_chat: Option<&str>, _max_depth: u32) -> Result<AxNode, PlatformError> {
        Err(PlatformError::Other(
            "inspect is only supported on macOS".into(),
        ))
    }
}