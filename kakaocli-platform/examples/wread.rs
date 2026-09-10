//! 실측: Windows 읽기 전체 경로 검증 — DEK 스캔 → 라이브 .edb 복사 → open_raw_key
//! → list_chats_windows / messages_windows. 읽기 전용, KakaoTalk에 아무것도 안 씀.

#[cfg(not(windows))]
fn main() {
    eprintln!("windows only");
}

#[cfg(windows)]
fn main() {
    use kakaocli_core::db_path;
    use kakaocli_db::Database;
    use kakaocli_platform::dek;

    let chat_data = db_path::windows_chat_data_path();
    let deks = dek::scan_deks(&chat_data).expect("scan_deks");
    println!("상주 DEK {}개\n", deks.len());

    // ── chats: chatListInfo.edb ──
    let hex = |k: &[u8; 32]| -> String { k.iter().map(|b| format!("{:02x}", b)).collect() };
    if let Some(dk) = deks.get("chatListInfo.edb") {
        let tmp = copy_to_temp(&chat_data.join("chatListInfo.edb")).expect("copy room");
        match Database::open_raw_key(&tmp, &hex(dk), 0) {
            Ok(db) => match db.list_chats_windows(15) {
                Ok(chats) => {
                    println!("=== 채팅방 {}개 (chatRoomList) ===", chats.len());
                    for c in &chats {
                        println!(
                            "  chatId={:<20} type={:?} unread={} title_len={}",
                            c.id,
                            c.chat_type,
                            c.unread_count,
                            c.display_name.chars().count()
                        );
                    }
                }
                Err(e) => println!("list_chats_windows err: {}", e),
            },
            Err(e) => println!("open chatListInfo err: {}", e),
        }
    } else {
        println!("chatListInfo.edb DEK 없음 (앱에서 목록이 로드 안 됨?)");
    }

    // ── messages: 첫 chatLogs_* DEK ──
    if let Some(name) = deks.keys().find(|k| k.starts_with("chatLogs_")) {
        let chat_id: i64 = name
            .trim_start_matches("chatLogs_")
            .trim_end_matches(".edb")
            .parse()
            .unwrap_or(0);
        let dk = deks[name];
        let tmp = copy_to_temp(&chat_data.join(name)).expect("copy log");
        match Database::open_raw_key(&tmp, &hex(&dk), 0) {
            Ok(db) => match db.messages_windows(chat_id, None, 5) {
                Ok(msgs) => {
                    println!("\n=== {} 최근 메시지 {}개 (chatLogs) ===", name, msgs.len());
                    for m in &msgs {
                        // 내용은 길이만 (프라이버시). 복호화 성공 = text 존재.
                        println!(
                            "  logId={:<18} author={:<18} sendAt={} type={:?} len={}",
                            m.id,
                            m.sender_id,
                            m.created_at,
                            m.message_type,
                            m.text.as_deref().map(|s| s.chars().count()).unwrap_or(0)
                        );
                    }
                }
                Err(e) => println!("messages_windows err: {}", e),
            },
            Err(e) => println!("open chatLogs err: {}", e),
        }
    }
}

/// 라이브 .edb(+ -wal/-shm)를 임시 폴더로 복사. KakaoTalk이 파일을 락하므로 읽기
/// 전에 복사한다. 원본은 건드리지 않는다.
#[cfg(windows)]
fn copy_to_temp(src: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let name = src.file_name().unwrap();
    let dir = std::env::temp_dir().join("kakaocli_wread");
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(name);
    std::fs::copy(src, &dst)?;
    for ext in ["-wal", "-shm"] {
        let s = src.with_file_name(format!("{}{}", name.to_string_lossy(), ext));
        if s.exists() {
            let _ = std::fs::copy(&s, dir.join(s.file_name().unwrap()));
        }
    }
    Ok(dst)
}
