//! Windows DEK 스캐너 실험 도구 (MoniKa 알고리즘 이식).
//!
//! 실행 중인 KakaoTalk.exe 메모리에서 SQLCipher DEK 후보를 `0x88` 앵커 기반으로
//! 수집하고, 각 후보를 AES-256-ECB page-1 헤더 지문 오라클로 검증한다. HMAC/전체
//! SQLCipher 없이 후보당 AES 1블록으로 판별한다.
//!
//! 참고 구현: MoniKa (github.com/maxswjeon/MoniKa) — scanner.cpp / oracle.cpp.
//! 데이터 포맷 상호운용을 위해 알고리즘을 Rust로 재구현.
//!
//! 사용:  KAKAO_EDB=<path to a .edb>  cargo run --example dek_scan

#[cfg(not(windows))]
fn main() {
    eprintln!("dek_scan is Windows-only.");
}

#[cfg(windows)]
fn main() {
    let edb = std::env::var("KAKAO_EDB")
        .ok()
        .or_else(|| std::env::args().nth(1))
        .unwrap_or_else(|| {
            eprintln!("set KAKAO_EDB=<path to a .edb> (or pass as argv[1])");
            std::process::exit(2);
        });
    win::run(&edb);
}

#[cfg(windows)]
mod win {
    use std::collections::HashSet;
    use std::ffi::c_void;
    use std::mem::size_of;

    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockDecrypt, KeyInit};
    use aes::Aes256;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Memory::{
        VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    // ── MoniKa 상수 (common.hpp) ──────────────────────────────
    const TARGET: &str = "KakaoTalk.exe";
    const PAGE_SIZE: usize = 4096;
    const DEK_LEN: usize = 32;
    const ANCHOR: u8 = 0x88;
    const ANCHOR_TAG_MAX: u8 = 8;
    const DEK_OFFSETS: [usize; 4] = [1, 16, 49, 56];
    const DEK_MAX_OFFSET: usize = 56;
    const RESERVED_CANDS: [usize; 6] = [80, 48, 64, 16, 32, 96];

    // Win32 protection bits (mbi.Protect & 0xFF)
    const PAGE_GUARD: u32 = 0x100;
    const READABLE: [u32; 6] = [0x02, 0x04, 0x08, 0x20, 0x40, 0x80];
    // PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY, EXECUTE_READ/READWRITE/WRITECOPY

    /// 한 .edb의 page-1 지문 재료.
    struct EdbProbe {
        name: String,
        c0: [u8; 16],
        ivs: Vec<(usize, [u8; 16])>, // (reserved R, per-page IV)
    }

    /// page-1 헤더 지문 오라클 (oracle.cpp 이식). 여러 .edb를 동시에 검사.
    struct Oracle {
        files: Vec<EdbProbe>,
    }

    fn probe_from_head(name: String, head: &[u8]) -> Option<EdbProbe> {
        if head.len() < PAGE_SIZE {
            return None;
        }
        let mut c0 = [0u8; 16];
        c0.copy_from_slice(&head[16..32]);
        let mut ivs = Vec::new();
        for &r in &RESERVED_CANDS {
            let start = PAGE_SIZE - r;
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&head[start..start + 16]);
            ivs.push((r, iv));
        }
        Some(EdbProbe { name, c0, ivs })
    }

    impl Oracle {
        /// KAKAO_EDB(단일 파일) 또는 KAKAO_EDB_DIR(디렉터리 재귀)에서 .edb 로드.
        fn load(target: &str) -> Oracle {
            let mut files = Vec::new();
            let p = std::path::Path::new(target);
            if p.is_dir() {
                collect_edb(p, &mut files);
            } else if let Ok(head) = std::fs::read(p) {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(target).to_string();
                if let Some(pr) = probe_from_head(name, &head) {
                    files.push(pr);
                }
            }
            Oracle { files }
        }

        /// AES-256-ECB(c0) XOR iv == SQLite header[16..24]? 맞으면 Some((file, R)).
        fn test_key(&self, key: &[u8; 32]) -> Option<(&str, usize)> {
            let cipher = Aes256::new(GenericArray::from_slice(key));
            for f in &self.files {
                let mut block = *GenericArray::from_slice(&f.c0);
                cipher.decrypt_block(&mut block);
                let p = block;
                for (r, iv) in &f.ivs {
                    let x = |k: usize| p[k] ^ iv[k];
                    if x(4) as usize != *r {
                        continue; // reserved-bytes discriminator (SQLite header[20])
                    }
                    if x(0) == 0x10
                        && x(1) == 0x00
                        && (x(2) == 1 || x(2) == 2)
                        && (x(3) == 1 || x(3) == 2)
                        && x(5) == 0x40
                        && x(6) == 0x20
                        && x(7) == 0x20
                    {
                        return Some((&f.name, *r));
                    }
                }
            }
            None
        }
    }

    fn collect_edb(dir: &std::path::Path, out: &mut Vec<EdbProbe>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                collect_edb(&path, out);
            } else if path.extension().and_then(|x| x.to_str()) == Some("edb") {
                if let Ok(head) = std::fs::read(&path) {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
                    if let Some(pr) = probe_from_head(name, &head) {
                        out.push(pr);
                    }
                }
            }
        }
    }

    pub fn run(edb: &str) {
        let oracle = Oracle::load(edb);
        if oracle.files.is_empty() {
            eprintln!("no readable .edb page-1 at: {}", edb);
            std::process::exit(1);
        }
        println!("오라클 로드: {}개 .edb", oracle.files.len());

        let pid = match find_pid(TARGET) {
            Some(p) => p,
            None => {
                eprintln!("{} is not running", TARGET);
                std::process::exit(1);
            }
        };
        println!("KakaoTalk.exe pid = {}", pid);

        let handle = match unsafe {
            OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid)
        } {
            Ok(h) => h,
            Err(e) => {
                eprintln!("OpenProcess failed: {e} (관리자 권한 / 동일 무결성 필요할 수 있음)");
                std::process::exit(1);
            }
        };

        let mut tested: HashSet<[u8; 32]> = HashSet::new();
        let mut cands = 0usize;
        let mut hit: Option<([u8; 32], String, usize)> = None;

        walk_anchored(handle, |key| {
            cands += 1;
            if !tested.insert(*key) {
                return false;
            }
            if let Some((name, r)) = oracle.test_key(key) {
                hit = Some((*key, name.to_string(), r));
                return true; // stop
            }
            false
        });

        unsafe {
            let _ = CloseHandle(handle);
        }

        match hit {
            Some((key, name, r)) => {
                println!("\n✅ DEK 발견! (매칭 파일 {}, reserved={})", name, r);
                println!("   DEK : {}", hex_encode(&key));
                println!("   후보 {}개 검사, 고유 {}개", cands, tested.len());
            }
            None => {
                println!(
                    "\n❌ DEK 없음. 앵커 후보 {}개(고유 {}개) 검사, {}개 .edb 어느 것도 못 엶.",
                    cands,
                    tested.len(),
                    oracle.files.len()
                );
                println!("   → 채팅방이 하나도 안 열려 있으면 DEK가 메모리에 없을 수 있음.");
            }
        }
    }

    fn find_pid(name: &str) -> Option<u32> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut entry = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = None;
            if Process32FirstW(snap, &mut entry).is_ok() {
                loop {
                    let exe = String::from_utf16_lossy(&entry.szExeFile);
                    if exe.trim_end_matches('\0').eq_ignore_ascii_case(name) {
                        found = Some(entry.th32ProcessID);
                        break;
                    }
                    if Process32NextW(snap, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
            found
        }
    }

    fn region_interesting(mbi: &MEMORY_BASIC_INFORMATION) -> bool {
        if mbi.State != MEM_COMMIT || mbi.Type != MEM_PRIVATE {
            return false;
        }
        let protect = mbi.Protect.0;
        if protect & PAGE_GUARD != 0 {
            return false;
        }
        READABLE.contains(&(protect & 0xFF))
    }

    /// 모든 관심 영역을 1MB 청크(overlap)로 훑어 0x88 앵커 후보를 perCand로 넘긴다.
    /// perCand가 true를 반환하면 중단.
    fn walk_anchored(handle: HANDLE, mut per_cand: impl FnMut(&[u8; 32]) -> bool) {
        const CHUNK: usize = 1024 * 1024;
        const OVERLAP: usize = DEK_MAX_OFFSET + DEK_LEN + 1;
        const MAXADDR: usize = 0x7FFF_FFFF_0000;

        let mut addr: usize = 0;
        let mut mbi = MEMORY_BASIC_INFORMATION::default();

        while addr < MAXADDR {
            let got = unsafe {
                VirtualQueryEx(
                    handle,
                    Some(addr as *const c_void),
                    &mut mbi,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if got == 0 {
                break;
            }
            let base = mbi.BaseAddress as usize;
            let size = mbi.RegionSize;
            if base == 0 && size == 0 {
                break;
            }

            if region_interesting(&mbi) && size >= DEK_LEN + 2 {
                let mut pos = 0usize;
                while pos < size {
                    let read_pos = if pos > 0 { pos - OVERLAP } else { 0 };
                    let want = (CHUNK + if pos > 0 { OVERLAP } else { 0 }).min(size - read_pos);
                    let mut buf = vec![0u8; want];
                    let mut read: usize = 0;
                    let ok = unsafe {
                        ReadProcessMemory(
                            handle,
                            (base + read_pos) as *const c_void,
                            buf.as_mut_ptr() as *mut c_void,
                            want,
                            Some(&mut read),
                        )
                    };
                    if ok.is_ok() && read >= DEK_LEN + 2 {
                        let d = &buf[..read];
                        let mut i = 1usize;
                        while i + DEK_MAX_OFFSET + DEK_LEN <= read {
                            if d[i] == ANCHOR && d[i - 1] < ANCHOR_TAG_MAX {
                                for &off in &DEK_OFFSETS {
                                    let key_slice = &d[i + off..i + off + DEK_LEN];
                                    let zeros = key_slice.iter().filter(|&&b| b == 0).count();
                                    if zeros > 2 {
                                        continue;
                                    }
                                    let mut key = [0u8; 32];
                                    key.copy_from_slice(key_slice);
                                    if per_cand(&key) {
                                        return;
                                    }
                                }
                            }
                            i += 1;
                        }
                    }
                    pos += CHUNK;
                }
            }

            match base.checked_add(size) {
                Some(n) if n > addr => addr = n,
                _ => break,
            }
        }
    }

    fn hex_encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }
}
