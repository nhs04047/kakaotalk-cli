//! sync 체크포인트: 스코프별 마지막 처리 logId를 `~/.kakaocli/sync.json`에 저장.
//!
//! 스코프 키:
//! - 전체 방: `"global"`
//! - 특정 방(`--chat`): `"chat:<chat_id>"`
//!
//! 두 스코프가 서로의 진행 상태를 덮어쓰지 않도록 키를 분리한다. 파일이 없거나
//! 손상됐으면 "미상(첫 실행)"으로 간주하고 새로 만든다 — 에러로 중단하지 않는다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db_path::config_dir;

/// 체크포인트 파일 경로 (`~/.kakaocli/sync.json`).
fn checkpoint_path() -> PathBuf {
    config_dir().join("sync.json")
}

/// 스코프 키 문자열. `None`이면 전체 방, `Some(id)`면 해당 방.
pub fn scope_key(chat_id: Option<i64>) -> String {
    match chat_id {
        Some(id) => format!("chat:{id}"),
        None => "global".to_string(),
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Checkpoints(BTreeMap<String, i64>);

fn load_from(path: &Path, chat_id: Option<i64>) -> Option<i64> {
    let content = std::fs::read_to_string(path).ok()?;
    let cp: Checkpoints = serde_json::from_str(&content).ok()?;
    cp.0.get(&scope_key(chat_id)).copied()
}

fn save_to(path: &Path, chat_id: Option<i64>, last_log_id: i64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Merge into whatever is already there so the other scope's value survives.
    let mut cp: Checkpoints = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    cp.0.insert(scope_key(chat_id), last_log_id);
    let json = serde_json::to_string_pretty(&cp)
        .map_err(std::io::Error::other)?;
    std::fs::write(path, json)
}

/// 스코프의 마지막 처리 logId를 읽는다. 미상/첫 실행/에러 시 `None`.
pub fn load(chat_id: Option<i64>) -> Option<i64> {
    load_from(&checkpoint_path(), chat_id)
}

/// 스코프의 마지막 처리 logId를 저장한다 (best-effort, 기존 값 병합).
pub fn save(chat_id: Option<i64>, last_log_id: i64) -> std::io::Result<()> {
    save_to(&checkpoint_path(), chat_id, last_log_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn test_scope_key() {
        assert_eq!(scope_key(None), "global");
        assert_eq!(scope_key(Some(42)), "chat:42");
    }

    #[test]
    fn test_save_load_roundtrip() {
        let path = temp_file("kakaocli_sync_roundtrip.json");
        let _ = std::fs::remove_file(&path);

        assert_eq!(load_from(&path, None), None); // missing file → None

        save_to(&path, None, 1500).unwrap();
        save_to(&path, Some(123), 90).unwrap();

        assert_eq!(load_from(&path, None), Some(1500));
        assert_eq!(load_from(&path, Some(123)), Some(90));
        // Scopes don't clobber each other.
        save_to(&path, None, 1600).unwrap();
        assert_eq!(load_from(&path, None), Some(1600));
        assert_eq!(load_from(&path, Some(123)), Some(90));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_corrupt_file_is_none() {
        let path = temp_file("kakaocli_sync_corrupt.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        assert_eq!(load_from(&path, None), None);
        // save over a corrupt file still succeeds (starts fresh).
        save_to(&path, None, 5).unwrap();
        assert_eq!(load_from(&path, None), Some(5));
        let _ = std::fs::remove_file(&path);
    }
}
