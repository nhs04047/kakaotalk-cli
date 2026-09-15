//! 실측(읽기 전용): KakaoTalk 메인 창(제목=카카오톡)의 자식 컨트롤을
//! class / dlg id / 가시성 / 텍스트 / 위치로 덤프. send 자동 열기용 검색
//! 컨트롤을 정확히 식별하기 위한 조사 도구. UI를 조작하지 않는다.
//!
//! 사용: cargo run --example wchild   (또는 target\debug\examples\wchild.exe)

#[cfg(not(windows))]
fn main() {
    eprintln!("windows only");
}

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(windows)]
mod win {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GetClassNameW, GetDlgCtrlID, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible,
    };

    pub fn run() {
        let pid = match kakaocli_platform::dek::find_kakao_pid() {
            Some(p) => p,
            None => {
                eprintln!("KakaoTalk.exe not running");
                return;
            }
        };
        println!("KakaoTalk pid = {}", pid);

        // 모든 top-level EVA 창 나열 (메인/대화상자/채팅창 구분용).
        let tops = enum_top(pid);
        println!("\n=== top-level 창 (pid {}) ===", pid);
        for (h, class, title, vis) in &tops {
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(*h, &mut r);
            }
            println!(
                "  hwnd=0x{:X} vis={} rect=({},{},{},{}) {}x{} class='{}' title='{}'",
                h.0 as usize,
                vis,
                r.left,
                r.top,
                r.right,
                r.bottom,
                r.right - r.left,
                r.bottom - r.top,
                class,
                title
            );
        }

        // 각 top-level 창의 자식 컨트롤 덤프.
        for (h, _, title, _) in &tops {
            println!("\n=== 자식 컨트롤: '{}' (0x{:X}) ===", title, h.0 as usize);
            let kids = enum_children(*h);
            for (ch, class, id, vis, text, r) in kids {
                println!(
                    "  id={:<6} vis={} class='{}' text='{}' rect=({},{},{},{}) hwnd=0x{:X}",
                    id,
                    vis,
                    class,
                    truncate(&text, 30),
                    r.left,
                    r.top,
                    r.right,
                    r.bottom,
                    ch.0 as usize
                );
            }
        }
    }

    fn enum_top(pid: u32) -> Vec<(HWND, String, String, bool)> {
        struct Ctx {
            pid: u32,
            out: Vec<(HWND, String, String, bool)>,
        }
        let mut ctx = Ctx {
            pid,
            out: Vec::new(),
        };
        unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
            let ctx = &mut *(l.0 as *mut Ctx);
            let mut wpid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut wpid));
            if wpid == ctx.pid {
                let class = class_name(h);
                let title = window_text(h);
                let vis = IsWindowVisible(h).as_bool();
                if !title.is_empty() || class.starts_with("EVA") {
                    ctx.out.push((h, class, title, vis));
                }
            }
            BOOL(1)
        }
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
        }
        ctx.out
    }

    fn enum_children(parent: HWND) -> Vec<(HWND, String, i32, bool, String, RECT)> {
        struct Ctx {
            out: Vec<(HWND, String, i32, bool, String, RECT)>,
        }
        let mut ctx = Ctx { out: Vec::new() };
        unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
            let ctx = &mut *(l.0 as *mut Ctx);
            let id = GetDlgCtrlID(h);
            let class = class_name(h);
            let text = window_text(h);
            let vis = IsWindowVisible(h).as_bool();
            let mut r = RECT::default();
            let _ = GetWindowRect(h, &mut r);
            ctx.out.push((h, class, id, vis, text, r));
            BOOL(1)
        }
        unsafe {
            let _ = EnumChildWindows(
                Some(parent),
                Some(cb),
                LPARAM(&mut ctx as *mut Ctx as isize),
            );
        }
        ctx.out
    }

    fn window_text(h: HWND) -> String {
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn class_name(h: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn truncate(s: &str, n: usize) -> String {
        let t: String = s.chars().take(n).collect();
        if s.chars().count() > n {
            format!("{}…", t)
        } else {
            t
        }
    }
}
