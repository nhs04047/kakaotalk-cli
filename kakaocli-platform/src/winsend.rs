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
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEINPUT, VIRTUAL_KEY, VK_CONTROL, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumChildWindows, EnumWindows, GetDlgCtrlID, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, SetCursorPos,
    SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
};

use crate::PlatformError;

const RICHEDIT_INPUT_ID: i32 = 1006;
/// 메인 창(제목=카카오톡)의 검색 입력창(표준 Edit) dlg id.
const SEARCH_EDIT_ID: i32 = 100;
const VK_A: u16 = 0x41;

/// 일반 채팅(친구/그룹) 전송. 열린 창이 없으면 메인 창 검색으로 자동 열기.
pub fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError> {
    let pid = crate::dek::find_kakao_pid().ok_or(PlatformError::AppNotAvailable)?;

    let mut windows = enum_chat_windows(pid);
    dbg_send(|| format!("초기 열린 채팅창: {:?}", titles(&windows)));
    if !windows.iter().any(|(_, t)| title_matches(t, chat_name)) {
        dbg_send(|| format!("'{}' 매칭 창 없음 → 검색 자동 열기 시도", chat_name));
        open_chat(pid, chat_name)?;
        windows = enum_chat_windows(pid);
        dbg_send(|| format!("자동 열기 후 채팅창: {:?}", titles(&windows)));
    }
    deliver_matched(&windows, chat_name, text)
}

/// 자기채팅(나와의 채팅) 전송. 자기채팅은 이름 검색에 안 뜨므로, 친구 탭
/// 최상단 프로필 행(=나와의 채팅)을 더블클릭해 연다.
pub fn send_self_message(self_title: &str, text: &str) -> Result<(), PlatformError> {
    let pid = crate::dek::find_kakao_pid().ok_or(PlatformError::AppNotAvailable)?;

    let mut windows = enum_chat_windows(pid);
    dbg_send(|| format!("초기 열린 채팅창: {:?}", titles(&windows)));
    if !windows.iter().any(|(_, t)| title_matches(t, self_title)) {
        dbg_send(|| "자기채팅 창 없음 → 친구 탭 최상단 더블클릭으로 열기".to_string());
        open_self_chat(pid)?;
        windows = enum_chat_windows(pid);
        dbg_send(|| format!("자기채팅 열기 후 채팅창: {:?}", titles(&windows)));
    }
    deliver_matched(&windows, self_title, text)
}

fn titles(windows: &[(HWND, String)]) -> Vec<&str> {
    windows.iter().map(|(_, t)| t.as_str()).collect()
}

/// windows에서 expected와 매칭되는 단일 창을 찾아 제목 재검증 후 전송.
fn deliver_matched(
    windows: &[(HWND, String)],
    expected: &str,
    text: &str,
) -> Result<(), PlatformError> {
    let matched: Vec<&(HWND, String)> = windows
        .iter()
        .filter(|(_, t)| title_matches(t, expected))
        .collect();

    let (hwnd, title) = match matched.as_slice() {
        [] => {
            return Err(PlatformError::Other(format!(
                "'{}' 채팅창을 열지 못했습니다 — KakaoTalk에서 해당 채팅방을 직접 연 뒤 다시 시도하세요.",
                expected
            )))
        }
        [one] => (one.0, one.1.clone()),
        many => {
            return Err(PlatformError::AmbiguousChatName {
                name: expected.to_string(),
                matches: many.iter().map(|(_, t)| t.clone()).collect(),
            })
        }
    };

    // 안전: 전송 직전 제목 재확인.
    if !title_matches(&title, expected) {
        return Err(PlatformError::ChatVerificationFailed {
            expected: expected.to_string(),
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
    dbg_send(|| format!("메인 창 발견: hwnd=0x{:X}", main.0 as usize));
    // 트레이/최소화 상태여도 창을 앞으로 복원 (AttachThreadInput로 포그라운드 락 우회).
    restore_foreground(main);
    sleep(Duration::from_millis(500));

    let search = find_child_by_id(main, SEARCH_EDIT_ID).ok_or_else(|| {
        PlatformError::UiError("메인 창 검색 입력창(Edit)을 찾지 못했습니다".into())
    })?;
    let r = window_rect(search);
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    dbg_send(|| {
        format!(
            "검색 Edit hwnd=0x{:X} rect=({},{},{},{}) {}x{}",
            search.0 as usize, r.left, r.top, r.right, r.bottom, w, h
        )
    });
    // 검색창이 접혀 있으면(0 크기) 클릭 대상이 없다 — 안내 후 중단(오발송/오입력 방지).
    if w <= 0 || h <= 0 {
        return Err(PlatformError::Other(
            "메인 창 검색 입력창이 접혀 있습니다 — KakaoTalk 메인 창에서 검색(돋보기)이 보이는 상태로 두고 다시 시도하세요."
                .into(),
        ));
    }

    // 커스텀 EVA edit은 SetFocus가 잘 안 먹으므로 실제 클릭으로 포커스.
    click_center(search);
    sleep(Duration::from_millis(200));

    // 기존 검색어 제거 후 방 이름 입력.
    press_ctrl_a();
    sleep(Duration::from_millis(60));
    type_unicode(chat_name);
    dbg_send(|| format!("검색어 입력 완료: '{}'", chat_name));
    // 검색 결과가 채워질 시간을 준 뒤 Enter로 상단 결과 열기.
    sleep(Duration::from_millis(800));
    press_enter();
    dbg_send(|| "Enter 입력(상단 결과 열기)".to_string());
    sleep(Duration::from_millis(1000));
    Ok(())
}

/// 트레이/숨김 상태의 창을 화면 앞으로 복원한다. AttachThreadInput으로 포그라운드
/// 락을 우회해 SetForegroundWindow가 실제로 먹도록 한다.
fn restore_foreground(hwnd: HWND) {
    unsafe {
        let mut tpid = 0u32;
        let target = GetWindowThreadProcessId(hwnd, Some(&mut tpid));
        let me = GetCurrentThreadId();
        let _ = AttachThreadInput(me, target, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = AttachThreadInput(me, target, false);
    }
}

fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut r);
    }
    r
}

fn mouse_input(flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// (x,y) 화면 좌표에 좌클릭 한 번.
fn click_point(x: i32, y: i32) {
    unsafe {
        let _ = SetCursorPos(x, y);
    }
    sleep(Duration::from_millis(40));
    unsafe {
        SendInput(
            &[
                mouse_input(MOUSEEVENTF_LEFTDOWN),
                mouse_input(MOUSEEVENTF_LEFTUP),
            ],
            size_of::<INPUT>() as i32,
        );
    }
}

/// (x,y) 화면 좌표에 더블클릭.
fn double_click_point(x: i32, y: i32) {
    click_point(x, y);
    sleep(Duration::from_millis(80));
    unsafe {
        SendInput(
            &[
                mouse_input(MOUSEEVENTF_LEFTDOWN),
                mouse_input(MOUSEEVENTF_LEFTUP),
            ],
            size_of::<INPUT>() as i32,
        );
    }
}

/// 컨트롤 중앙을 실제 좌클릭해 포커스를 준다.
fn click_center(hwnd: HWND) {
    let r = window_rect(hwnd);
    click_point((r.left + r.right) / 2, (r.top + r.bottom) / 2);
}

/// 자기채팅(나와의 채팅)을 연다: 친구 탭 최상단 프로필 행을 더블클릭.
/// 자기채팅은 이름 검색에 안 뜨므로 검색 경로를 쓸 수 없다.
fn open_self_chat(pid: u32) -> Result<(), PlatformError> {
    let main = find_main_window(pid).ok_or_else(|| {
        PlatformError::Other("KakaoTalk 메인 창을 찾지 못했습니다 (로그인 상태인지 확인).".into())
    })?;
    dbg_send(|| format!("메인 창 발견: hwnd=0x{:X}", main.0 as usize));
    restore_foreground(main);
    sleep(Duration::from_millis(500));

    // 친구 리스트(ContactListCtrl)를 창 텍스트로 식별. 최상단 행이 나와의 채팅.
    let list = find_child_by_text_prefix(main, "ContactListCtrl").ok_or_else(|| {
        PlatformError::Other(
            "친구 목록을 찾지 못했습니다 — KakaoTalk 친구 탭을 연 상태로 다시 시도하세요.".into(),
        )
    })?;
    if !unsafe { IsWindowVisible(list) }.as_bool() {
        return Err(PlatformError::Other(
            "친구 목록이 보이지 않습니다 — KakaoTalk에서 친구 탭을 연 뒤 다시 시도하세요.".into(),
        ));
    }
    let r = window_rect(list);
    let x = (r.left + r.right) / 2;
    let y = r.top + 32; // 최상단 행(본인 프로필 = 나와의 채팅) 중심 근처
    dbg_send(|| {
        format!(
            "친구 리스트 rect=({},{},{},{}) → 최상단 행 더블클릭 ({},{})",
            r.left, r.top, r.right, r.bottom, x, y
        )
    });
    double_click_point(x, y);
    sleep(Duration::from_millis(1200));
    Ok(())
}

/// 부모 창의 자식 중 창 텍스트가 prefix로 시작하는 첫 컨트롤을 찾는다.
fn find_child_by_text_prefix(parent: HWND, prefix: &str) -> Option<HWND> {
    struct Ctx<'a> {
        prefix: &'a str,
        found: Option<HWND>,
    }
    let mut ctx = Ctx {
        prefix,
        found: None,
    };
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l.0 as *mut Ctx);
        if window_text(h).starts_with(ctx.prefix) {
            ctx.found = Some(h);
            return BOOL(0);
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

/// KAKAOCLI_SEND_DEBUG가 설정돼 있으면 send 자동화 단계를 stderr로 출력.
fn dbg_send(msg: impl FnOnce() -> String) {
    if std::env::var_os("KAKAOCLI_SEND_DEBUG").is_some() {
        eprintln!("[send] {}", msg());
    }
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
