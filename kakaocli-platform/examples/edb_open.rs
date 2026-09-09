//! 실험: 찾은 DEK로 shipping `kakaocli_db::Database::open_raw_key`가 실제 .edb를
//! 여는지 + 스키마(테이블) 확인. 여러 .edb에 대해 DEK 공유 여부도 점검.
//!
//! 사용:
//!   KAKAO_DEK=<64hex>  KAKAO_EDB=<file 또는 dir>  cargo run --example edb_open

use std::path::Path;

fn main() {
    let dek = std::env::var("KAKAO_DEK").expect("set KAKAO_DEK=<64hex>");
    let target = std::env::var("KAKAO_EDB").expect("set KAKAO_EDB=<file or dir>");
    let p = Path::new(&target);

    let mut files = Vec::new();
    if p.is_dir() {
        collect(p, &mut files);
    } else {
        files.push(p.to_path_buf());
    }
    files.sort();

    println!("DEK로 {}개 .edb 시도\n", files.len());
    for f in &files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        match kakaocli_db::Database::open_raw_key(f, &dek, 0) {
            Ok(db) => match db.verify_tables() {
                Ok(tables) => println!("  ✅ {:<32} tables: {}", name, tables.join(", ")),
                Err(e) => println!("  ⚠️  {:<32} opened but verify failed: {}", name, e),
            },
            Err(_) => println!("  ✗  {:<32} (이 DEK로 안 열림)", name),
        }
    }
}

fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().and_then(|x| x.to_str()) == Some("edb") {
                out.push(path);
            }
        }
    }
}
