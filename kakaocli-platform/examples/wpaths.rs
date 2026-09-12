//! 실측: Windows KakaoTalk 경로 발견 검증.
fn main() {
    #[cfg(windows)]
    {
        use kakaocli_core::db_path;
        println!("kakao base : {}", db_path::windows_kakao_base().display());
        println!("user dir   : {:?}", db_path::windows_user_dir());
        println!(
            "chat_data  : {}",
            db_path::windows_chat_data_path().display()
        );
        let edbs = db_path::windows_edb_files();
        println!("edb count  : {}", edbs.len());
        for p in edbs.iter().take(5) {
            println!(
                "   - {}",
                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            );
        }
    }
    #[cfg(not(windows))]
    println!("windows only");
}
