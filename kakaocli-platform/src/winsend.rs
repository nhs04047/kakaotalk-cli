//! Windows 메시지 전송 (UIAutomation 대신 Win32 컨트롤 직접 제어).
//!
//! KakaoTalk Windows는 채팅을 별도 창(EVA_Window_Dblclk, 제목=방이름)으로 열고,
//! 입력창은 표준 `RICHEDIT50W`(dlg id 1006)다. 열린 채팅창을 제목으로 찾아 검증한
//! 뒤 입력창에 포커스를 주고 유니코드 키 입력 + Enter로 전송한다.
//!
//! 안전: 전송 전 창 제목을 재확인(다른 방 오발송 방지). 채팅창이 열려 있지 않으면
//! 메인 창 검색창(dlg id 100)에 방 이름을 입력해 자동으로 연 뒤(open_chat),
//! 제목 재검증을 거쳐 전송한다.

use std::mem::size_of;
use std::thread::sleep;
use std::time::Duration;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetDlgCtrlID, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

use crate::PlatformError;

const RICHEDIT_INPUT_ID: i32 = 1006;
/// 메인 창(제목=카카오톡)의 검색 입력창(표준 Edit) dlg id.
const SEARCH_EDIT_ID: i32 = 100;
const VK_A: u16 = 0x41;

pub fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError> {
    let pid = crate::dek::find_kakao_pid().ok_or(PlatformError::AppNotAvailable)?;

    // 이미 열린 채팅창을 먼저 찾고, 없으면 메인 창 검색으로 자동 열기 시도.
    let mut windows = enum_chat_windows(pid);
    if !windows.iter().any(|(_, t)| title_matches(t, chat_name)) {
        open_chat(pid, chat_name)?;
        windows = enum_chat_windows(pid);
    }
    let matched: Vec<&(HWND, String)> = windows
        .iter()
        .filter(|(_, t)| title_matches(t, chat_name))
        .collect();

    let (hwnd, title) = match matched.as_slice() {
        [] => {
            return Err(PlatformError::Other(format!(
                "'{}' 채팅창을 열지 못했습니다 — 방 이름을 정확히 지정했는지 확인하거나 \
                 KakaoTalk에서 해당 채팅방을 직접 연 뒤 다시 시도하세요.",
                chat_name
            )))
        }
        [one] => (one.0, one.1.clone()),
        many => {
            return Err(PlatformError::AmbiguousChatName {
                name: chat_name.to_string(),
                matches: many.iter().map(|(_, t)| t.clone()).collect(),
            })
        }
    };

    // 안전: 전송 직전 제목 재확인.
    if !title_matches(&title, chat_name) {
        return Err(PlatformError::ChatVerificationFailed {
            expected: chat_name.to_string(),
            actual: title,
        });
    }

    let edit = find_richedit(hwnd)
        .ok_or_else(|| PlatformError::UiError("입력창(RichEdit)을 찾지 못했습니다".into()))?;

    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
        sleep(Duration::from_millis(250));
        let _ = SetFocus(Some(edit));
        sleep(Duration::from_millis(120));
    }

    let sanitized = text.replace(['\r', '\n'], " ");
    type_unicode(&sanitized);
    sleep(Duration::from_millis(120));
    press_enter();

    Ok(())
}

/// KakaoTalk pid의 채팅창(EVA_Window_Dblclk, 제목 있는 것, 메인창 제외)을 수집.
fn enum_chat_windows(pid: u32) -> Vec<(HWND, String)> {
    struct Ctx {
        pid: u32,
        out: Vec<(HWND, String)>,
    }
    let mut ctx = Ctx {
        pid,
        out: Vec::new(),
    };

    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l.0 as *mut Ctx);
        let mut wpid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut wpid));
        if wpid == ctx.pid && class_name(h) == "EVA_Window_Dblclk" {
            let t = window_text(h);
            if !t.is_empty() && t != "카카오톡" {
                ctx.out.push((h, t));
            }
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.out
}

/// 채팅창의 RichEdit 입력 컨트롤(dlg id 1006)을 찾는다.
fn find_richedit(parent: HWND) -> Option<HWND> {
    find_child_by_id(parent, RICHEDIT_INPUT_ID)
}

/// 부모 창의 자식 중 지정 dlg id를 가진 첫 컨트롤을 찾는다.
fn find_child_by_id(parent: HWND, id: i32) -> Option<HWND> {
    struct Ctx {
        id: i32,
        found: Option<HWND>,
    }
    let mut ctx = Ctx { id, found: None };

    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l.0 as *mut Ctx);
        if GetDlgCtrlID(h) == ctx.id {
            ctx.found = Some(h);
            return BOOL(0); // stop
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumChildWindows(
            Some(parent),
            Some(cb),
            LPARAM(&mut ctx as *mut Ctx as isize),
        );
    }
    ctx.found
}

/// 메인 창(EVA_Window_Dblclk, 제목=카카오톡)을 찾는다.
fn find_main_window(pid: u32) -> Option<HWND> {
    struct Ctx {
        pid: u32,
        found: Option<HWND>,
    }
    let mut ctx = Ctx { pid, found: None };

    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l.0 as *mut Ctx);
        let mut wpid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut wpid));
        if wpid == ctx.pid && class_name(h) == "EVA_Window_Dblclk" && window_text(h) == "카카오톡"
        {
            ctx.found = Some(h);
            return BOOL(0); // stop
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.found
}

/// 메인 창의 검색창(dlg id 100)에 방 이름을 입력하고 Enter로 상단 결과를 연다.
/// 자동 열기 후에도 send_message는 제목을 재검증하므로 오발송은 차단된다.
fn open_chat(pid: u32, chat_name: &str) -> Result<(), PlatformError> {
    let main = find_main_window(pid).ok_or_else(|| {
        PlatformError::Other("KakaoTalk 메인 창을 찾지 못했습니다 (로그인 상태인지 확인).".into())
    })?;
    let search = find_child_by_id(main, SEARCH_EDIT_ID).ok_or_else(|| {
        PlatformError::UiError("메인 창 검색 입력창(Edit)을 찾지 못했습니다".into())
    })?;

    unsafe {
        let _ = ShowWindow(main, SW_RESTORE);
        let _ = SetForegroundWindow(main);
        sleep(Duration::from_millis(250));
        let _ = SetFocus(Some(search));
        sleep(Duration::from_millis(120));
    }

    // 기존 검색어 제거 후 방 이름 입력.
    press_ctrl_a();
    sleep(Duration::from_millis(60));
    type_unicode(chat_name);
    // 검색 결과가 채워질 시간을 준 뒤 Enter로 상단 결과 열기.
    sleep(Duration::from_millis(700));
    press_enter();
    sleep(Duration::from_millis(900));
    Ok(())
}

fn title_matches(title: &str, query: &str) -> bool {
    let t = title.to_lowercase();
    let q = query.to_lowercase();
    t.contains(&q) || q.contains(&t)
}

fn window_text(h: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(h);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn class_name(h: HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n as usize])
}

/// 유니코드 문자열을 SendInput(KEYEVENTF_UNICODE)로 입력. 한글 포함 BMP 처리.
fn type_unicode(text: &str) {
    let mut inputs: Vec<INPUT> = Vec::new();
    for unit in text.encode_utf16() {
        inputs.push(key_unicode(unit, false));
        inputs.push(key_unicode(unit, true));
    }
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

fn key_unicode(scan: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key_vk(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let flags = if up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Ctrl+A (전체 선택) — 검색창의 기존 텍스트 제거용.
fn press_ctrl_a() {
    let seq = [
        key_vk(VK_CONTROL, false),
        key_vk(VIRTUAL_KEY(VK_A), false),
        key_vk(VIRTUAL_KEY(VK_A), true),
        key_vk(VK_CONTROL, true),
    ];
    unsafe {
        SendInput(&seq, size_of::<INPUT>() as i32);
    }
}

fn press_enter() {
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[down, up], size_of::<INPUT>() as i32);
    }
}
