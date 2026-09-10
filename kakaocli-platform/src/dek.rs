//! Windows DEK 스캐너: 실행 중 KakaoTalk.exe 메모리에서 SQLCipher DEK를 추출한다.
//!
//! Windows KakaoTalk은 공개 키유도가 동작하지 않아, 프로세스 메모리에서 32바이트
//! 원시 DEK를 포착하고 각 `.edb`의 page-1 헤더 지문으로 검증한다. DEK는 **파일별**
//! (per-database)이며 해당 채팅방이 열려 있어야 메모리에 상주한다.
//!
//! 알고리즘 참고: MoniKa (github.com/maxswjeon/MoniKa). 데이터 포맷 상호운용을 위해
//! Rust로 재구현. 읽기 전용 — 프로세스/파일에 아무것도 쓰지 않는다.

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, KeyInit};
use aes::Aes256;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{
    VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

use crate::PlatformError;

const TARGET: &str = "KakaoTalk.exe";
const PAGE_SIZE: usize = 4096;
const DEK_LEN: usize = 32;
const ANCHOR: u8 = 0x88;
const ANCHOR_TAG_MAX: u8 = 8;
const DEK_OFFSETS: [usize; 4] = [1, 16, 49, 56];
const DEK_MAX_OFFSET: usize = 56;
const RESERVED_CANDS: [usize; 6] = [80, 48, 64, 16, 32, 96];

const PAGE_GUARD: u32 = 0x100;
const READABLE: [u32; 6] = [0x02, 0x04, 0x08, 0x20, 0x40, 0x80];

/// page-1 헤더 지문 오라클: 한 후보 키가 어느 `.edb`를 여는지 판별.
struct EdbProbe {
    name: String,
    c0: [u8; 16],
    ivs: Vec<(usize, [u8; 16])>,
}

struct Oracle {
    files: Vec<EdbProbe>,
}

impl Oracle {
    /// `edb_dir` 아래 모든 `.edb`의 page-1 지문을 로드.
    fn load(edb_dir: &Path) -> Oracle {
        let mut files = Vec::new();
        collect(edb_dir, &mut files);
        Oracle { files }
    }

    /// AES-256-ECB(c0) XOR iv == SQLite header[16..24]? 맞으면 (파일명, R).
    fn test_key(&self, key: &[u8; 32]) -> Option<(&str, usize)> {
        let cipher = Aes256::new(GenericArray::from_slice(key));
        for f in &self.files {
            let mut block = *GenericArray::from_slice(&f.c0);
            cipher.decrypt_block(&mut block);
            let p = block;
            for (r, iv) in &f.ivs {
                let x = |k: usize| p[k] ^ iv[k];
                if x(4) as usize != *r {
                    continue;
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

fn collect(dir: &Path, out: &mut Vec<EdbProbe>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("edb") {
            if let Ok(head) = std::fs::read(&path) {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                if let Some(pr) = probe_from_head(name, &head) {
                    out.push(pr);
                }
            }
        }
    }
}

/// 실행 중 KakaoTalk.exe에서 상주 중인 DEK를 모두 수집한다.
/// 반환: `.edb 파일명 → 32바이트 DEK`. 열려 있지 않은 채팅방의 DEK는 없을 수 있다.
pub fn scan_deks(edb_dir: &Path) -> Result<HashMap<String, [u8; 32]>, PlatformError> {
    let oracle = Oracle::load(edb_dir);
    if oracle.files.is_empty() {
        return Err(PlatformError::Other(format!(
            "No .edb files under {}",
            edb_dir.display()
        )));
    }
    let total = oracle.files.len();

    let pid = find_pid(TARGET).ok_or(PlatformError::AppNotAvailable)?;
    let handle = unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid) }
        .map_err(|e| PlatformError::DekScanFailed(format!("OpenProcess: {e}")))?;

    let mut tested: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
    let mut found: HashMap<String, [u8; 32]> = HashMap::new();

    walk_anchored(handle, |key| {
        if !tested.insert(*key) {
            return false;
        }
        if let Some((name, _r)) = oracle.test_key(key) {
            found.entry(name.to_string()).or_insert(*key);
        }
        found.len() == total // stop once every file has a DEK
    });

    unsafe {
        let _ = CloseHandle(handle);
    }
    Ok(found)
}

/// 특정 `.edb` 하나의 DEK만 찾는다 (있으면).
pub fn scan_dek_for(edb_dir: &Path, file_name: &str) -> Result<Option<[u8; 32]>, PlatformError> {
    Ok(scan_deks(edb_dir)?.get(file_name).copied())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 라이브 `.edb`(+ `-wal`/`-shm`)를 임시 폴더로 복사한다. KakaoTalk이 파일을
/// 락하므로 읽기 전에 복사한다. **원본은 절대 건드리지 않는다(읽기 전용).**
pub fn copy_edb_to_temp(src: &Path) -> std::io::Result<std::path::PathBuf> {
    let name = src
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let dir = std::env::temp_dir().join("kakaocli");
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(name);
    std::fs::copy(src, &dst)?;
    for ext in ["-wal", "-shm"] {
        let sidecar = src.with_file_name(format!("{}{}", name.to_string_lossy(), ext));
        if sidecar.exists() {
            if let Some(fname) = sidecar.file_name() {
                let _ = std::fs::copy(&sidecar, dir.join(fname));
            }
        }
    }
    Ok(dst)
}

/// `chat_data/<file_name>`의 DEK를 스캔하고, 락 회피를 위해 임시 복사한 뒤
/// SQLCipher raw-key로 연다. DEK가 메모리에 없으면(채팅방 미개방) 안내 에러.
pub fn open_edb(
    chat_data: &Path,
    file_name: &str,
    my_uid: i64,
) -> Result<kakaocli_db::Database, PlatformError> {
    let dek = scan_dek_for(chat_data, file_name)?.ok_or_else(|| {
        PlatformError::Other(format!(
            "{} 의 DEK가 메모리에 없습니다 — KakaoTalk에서 해당 항목/채팅방을 한 번 열어주세요.",
            file_name
        ))
    })?;
    let src = chat_data.join(file_name);
    let tmp = copy_edb_to_temp(&src)
        .map_err(|e| PlatformError::Other(format!("복사 실패({}): {}", file_name, e)))?;
    kakaocli_db::Database::open_raw_key(&tmp, &hex_encode(&dek), my_uid)
        .map_err(|e| PlatformError::Other(format!("복호화 실패({}): {}", file_name, e)))
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

/// 관심 영역을 1MB 청크(overlap)로 훑어 0x88 앵커 후보를 per_cand로 넘긴다.
/// per_cand가 true를 반환하면 중단.
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
                                let slice = &d[i + off..i + off + DEK_LEN];
                                if slice.iter().filter(|&&b| b == 0).count() > 2 {
                                    continue;
                                }
                                let mut key = [0u8; 32];
                                key.copy_from_slice(slice);
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
