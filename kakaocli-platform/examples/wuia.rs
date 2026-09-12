//! 실측: KakaoTalk Windows UIAutomation 트리 조사 (send 구현용). 읽기 전용 —
//! UI를 조작하지 않고 구조만 덤프한다.
//!
//! 사용:  cargo run --example wuia                 (top-level 창 + 메인창 트리)
//!        KAKAO_UIA_DEPTH=6 cargo run --example wuia

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
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
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

        let hwnds = enum_top_windows();
        let mut main_hwnd: Option<HWND> = None;
        for h in hwnds {
            let mut wpid = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut wpid)) };
            if wpid != pid {
                continue;
            }
            let visible = unsafe { IsWindowVisible(h) }.as_bool();
            let title = window_text(h);
            let class = window_class(h);
            if !title.is_empty() || visible {
                println!(
                    "  HWND=0x{:X} vis={} class='{}' title='{}'",
                    h.0 as usize, visible, class, title
                );
                if visible && !title.is_empty() && main_hwnd.is_none() {
                    main_hwnd = Some(h);
                }
            }
        }

        // KAKAO_HWND=0x.... 로 특정 창 지정 가능.
        let hwnd = match std::env::var("KAKAO_HWND")
            .ok()
            .and_then(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        {
            Some(v) => HWND(v as *mut _),
            None => match main_hwnd {
                Some(h) => h,
                None => {
                    eprintln!("메인 창을 찾지 못했습니다.");
                    return;
                }
            },
        };

        let depth: u32 = std::env::var("KAKAO_UIA_DEPTH")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5);

        println!(
            "\n=== UIA 트리 (메인창 0x{:X}, depth {}) ===",
            hwnd.0 as usize, depth
        );
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let auto: IUIAutomation =
                match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("CUIAutomation 실패: {e}");
                        return;
                    }
                };
            let root = match auto.ElementFromHandle(hwnd) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("ElementFromHandle 실패: {e}");
                    return;
                }
            };
            let walker = match auto.ControlViewWalker() {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("ControlViewWalker 실패: {e}");
                    return;
                }
            };
            walk(&walker, &root, 0, depth);
        }
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, l: LPARAM) -> BOOL {
        let out = &mut *(l.0 as *mut Vec<HWND>);
        out.push(hwnd);
        BOOL(1)
    }

    fn enum_top_windows() -> Vec<HWND> {
        let mut out: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut out as *mut Vec<HWND> as isize));
        }
        out
    }

    fn window_text(h: HWND) -> String {
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn window_class(h: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n as usize])
    }

    unsafe fn walk(
        walker: &IUIAutomationTreeWalker,
        el: &IUIAutomationElement,
        depth: u32,
        max: u32,
    ) {
        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let ct = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
        let aid = el
            .CurrentAutomationId()
            .map(|b| b.to_string())
            .unwrap_or_default();
        let indent = "  ".repeat(depth as usize);
        // 이름/자동화ID가 있거나 컨테이너류만 노출 (노이즈 감소)
        println!(
            "{}[{}] name='{}' id='{}'",
            indent,
            control_type_name(ct),
            truncate(&name, 40),
            aid
        );

        if depth >= max {
            return;
        }
        if let Ok(first) = walker.GetFirstChildElement(el) {
            let mut cur = first;
            loop {
                walk(walker, &cur, depth + 1, max);
                match walker.GetNextSiblingElement(&cur) {
                    Ok(n) => cur = n,
                    Err(_) => break,
                }
            }
        }
    }

    fn truncate(s: &str, n: usize) -> String {
        let t: String = s.chars().take(n).collect();
        if s.chars().count() > n {
            format!("{}…", t)
        } else {
            t
        }
    }

    fn control_type_name(ct: i32) -> &'static str {
        match ct {
            50000 => "Button",
            50004 => "Edit",
            50008 => "List",
            50007 => "ListItem",
            50011 => "Text",
            50020 => "Pane",
            50026 => "Group",
            50032 => "Window",
            50033 => "Document",
            50024 => "ScrollBar",
            50003 => "ComboBox",
            50021 => "Tree",
            50022 => "TreeItem",
            50002 => "CheckBox",
            50019 => "MenuItem",
            50025 => "TabItem",
            _ => "?",
        }
    }
}
