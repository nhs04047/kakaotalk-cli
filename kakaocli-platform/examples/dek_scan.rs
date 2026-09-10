//! 실측: 실행 중 KakaoTalk.exe에서 상주 DEK를 모두 수집 (kakaocli_platform::dek).
//!
//! 사용:  cargo run --example dek_scan
//!        KAKAO_EDB=<chat_data dir> cargo run --example dek_scan   (경로 override)

#[cfg(not(windows))]
fn main() {
    eprintln!("dek_scan is Windows-only.");
}

#[cfg(windows)]
fn main() {
    use std::path::PathBuf;

    let dir = std::env::var("KAKAO_EDB")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(kakaocli_core::db_path::windows_chat_data_path);

    println!("chat_data: {}", dir.display());
    match kakaocli_platform::dek::scan_deks(&dir) {
        Ok(map) => {
            println!("상주 DEK {}개 발견:\n", map.len());
            let mut names: Vec<_> = map.keys().cloned().collect();
            names.sort();
            for name in names {
                let key = &map[&name];
                let hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
                println!("  {:<34} {}", name, hex);
            }
        }
        Err(e) => eprintln!("scan failed: {}", e),
    }
}
