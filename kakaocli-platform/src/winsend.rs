//! Windows 메시지 전송 (UIAutomation 대신 Win32 컨트롤 직접 제어).
//!
//! KakaoTalk Windows는 채팅을 별도 창(EVA_Window_Dblclk, 제목=방이름)으로 열고,
//! 입력창은 표준 `RICHEDIT50W`(dlg id 1006)다. 열린 채팅창을 제목으로 찾아 검증한
//! 뒤 입력창에 포커스를 주고 유니코드 키 입력 + Enter로 전송한다.
//!
//! 전송은 화면에 창을 띄우지 않는다: 대상 채팅창을 잠깐 화면 밖으로 옮겨 포그라운드로
//! 만들어 SendInput으로 입력·전송한 뒤 원위치로 되돌린다(KakaoTalk은 저수준 입력만
//! 전송으로 인정해 WM_SETTEXT 등 백그라운드 메시지로는 전송이 안 됨).
//!
//! 안전: 전송 직전 창 제목을 재확인(다른 방 오발송 방지). 일반 채팅은 "열린 창"에만
//! 전송한다(통합검색 자동 열기는 결과 행이 자동화 API에 노출되지 않아 "친구 추가"
//! 오작동 위험 → 미지원). 자기채팅은 친구 탭 최상단 프로필 더블클릭으로 연다.

use std::mem::size_of;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use windows::core::BOOL;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, POINT, RECT};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEINPUT, VIRTUAL_KEY, VK_CONTROL, VK_RETURN,
};
use windows::Win32::UI::Shell::DROPFILES;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumChildWindows, EnumWindows, GetDlgCtrlID, GetForegroundWindow,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SetCursorPos, SetForegroundWindow, SetWindowPos, ShowWindow, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SW_RESTORE, SW_SHOW,
};

use crate::PlatformError;

const RICHEDIT_INPUT_ID: i32 = 1006;

/// 일반 채팅(친구/그룹) 전송. 열린 창에만 전송한다(화면 없이 조용히).
///
/// 안 열린 방을 검색으로 자동 열기는 KakaoTalk 통합검색이 결과 최상단에 "친구 추가"
/// 제안을 넣고, 결과 행이 어떤 자동화 API(Win32 자식창/UIAutomation)에도 노출되지 않아
/// 정확히 "채팅방" 결과만 클릭할 수 없다(친구 추가 오작동). 그래서 자동 열기는 하지
/// 않고, 사용자가 방을 열면 무팝업으로 전송한다. (자기채팅은 send_self_message에서
/// 친구 탭 최상단 더블클릭이라는 안정적 경로로 연다.)
pub fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError> {
    let pid = crate::dek::find_kakao_pid().ok_or(PlatformError::AppNotAvailable)?;

    let windows = enum_chat_windows(pid);
    dbg_send(|| format!("초기 열린 채팅창: {:?}", titles(&windows)));
    if !windows.iter().any(|(_, t)| title_matches(t, chat_name)) {
        return Err(PlatformError::Other(format!(
            "'{}' 채팅창이 열려 있지 않습니다 — KakaoTalk에서 해당 채팅방을 먼저 연 뒤 다시 시도하세요. \
             (열려 있으면 화면 없이 조용히 전송됩니다.)",
            chat_name
        )));
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

    within_focused(hwnd, edit, || {
        let sanitized = text.replace(['\r', '\n'], " ");
        type_unicode(&sanitized);
        sleep(Duration::from_millis(120));
        press_enter();
        sleep(Duration::from_millis(200));
    });
    dbg_send(|| format!("전송 완료(화면 밖 입력): edit=0x{:X}", edit.0 as usize));

    Ok(())
}

/// 채팅창 입력 컨트롤에 포커스를 준 상태로 `f`를 실행한다(저수준 입력용).
///
/// KakaoTalk은 저수준 입력(SendInput)만 "전송"으로 인정하고, 이는 포그라운드 창에만
/// 들어간다. 화면에 안 보이게 하려고 채팅창을 잠깐 화면 밖으로 옮겨 포그라운드로 만든
/// 뒤 `f`로 입력하고, 원래 포그라운드 창과 채팅창 위치를 되돌린다.
fn within_focused(hwnd: HWND, edit: HWND, f: impl FnOnce()) {
    let prev = unsafe { GetForegroundWindow() };
    let orig = window_rect(hwnd);
    let was_min = unsafe { IsIconic(hwnd) }.as_bool();
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    foreground_offscreen(hwnd);
    unsafe {
        let _ = SetFocus(Some(edit));
    }
    sleep(Duration::from_millis(150));

    f();

    if (prev.0 as usize) != 0 && prev != hwnd {
        foreground_offscreen(prev);
    }
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            orig.left,
            orig.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        if was_min {
            let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_MINIMIZE);
        }
    }
}

/// 창을 위치 변경 없이(SW_RESTORE로 원위치 튀는 것 방지) 포그라운드로 만든다.
/// 화면 밖으로 옮긴 채팅창에 SendInput을 넣기 위한 용도.
fn foreground_offscreen(hwnd: HWND) {
    unsafe {
        let mut tpid = 0u32;
        let target = GetWindowThreadProcessId(hwnd, Some(&mut tpid));
        let me = GetCurrentThreadId();
        let _ = AttachThreadInput(me, target, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = AttachThreadInput(me, target, false);
    }
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

// ── 첨부(파일·이미지) 전송 ───────────────────────────────────

const CF_HDROP: u32 = 15;

/// 파일·이미지 전송(열린 채팅창에 클립보드 붙여넣기). 파일 경로들을 CF_HDROP로
/// 클립보드에 올린 뒤 입력창에 Ctrl+V → Enter. KakaoTalk은 복사된 파일 붙여넣기를
/// 첨부로 인식한다(이미지 파일은 사진으로). 창은 미리 열려 있어야 한다(텍스트와 동일).
/// `paths`가 `Some`이면 그 파일들을, `None`이면 현재 클립보드 내용을 전송한다.
pub fn send_attachment_message(
    chat_name: &str,
    paths: Option<&[PathBuf]>,
) -> Result<(), PlatformError> {
    let pid = crate::dek::find_kakao_pid().ok_or(PlatformError::AppNotAvailable)?;
    let windows = enum_chat_windows(pid);
    if !windows.iter().any(|(_, t)| title_matches(t, chat_name)) {
        return Err(PlatformError::Other(format!(
            "'{}' 채팅창이 열려 있지 않습니다 — KakaoTalk에서 해당 채팅방을 먼저 연 뒤 다시 시도하세요.",
            chat_name
        )));
    }
    deliver_attachment(&windows, chat_name, paths)
}

/// 자기채팅(나와의 채팅)으로 파일·이미지(또는 클립보드) 전송.
pub fn send_self_attachment(
    self_title: &str,
    paths: Option<&[PathBuf]>,
) -> Result<(), PlatformError> {
    let pid = crate::dek::find_kakao_pid().ok_or(PlatformError::AppNotAvailable)?;
    let mut windows = enum_chat_windows(pid);
    if !windows.iter().any(|(_, t)| title_matches(t, self_title)) {
        open_self_chat(pid)?;
        windows = enum_chat_windows(pid);
    }
    deliver_attachment(&windows, self_title, paths)
}

fn deliver_attachment(
    windows: &[(HWND, String)],
    expected: &str,
    paths: Option<&[PathBuf]>,
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
    // 안전: 전송 직전 제목 재확인(오발송 방지).
    if !title_matches(&title, expected) {
        return Err(PlatformError::ChatVerificationFailed {
            expected: expected.to_string(),
            actual: title,
        });
    }
    let edit = find_richedit(hwnd)
        .ok_or_else(|| PlatformError::UiError("입력창(RichEdit)을 찾지 못했습니다".into()))?;

    // 파일 지정 시 클립보드에 올린다(전역이라 포커스 전에). None이면 현재 클립보드 사용.
    if let Some(paths) = paths {
        set_clipboard_files(paths)?;
    }

    within_focused(hwnd, edit, || {
        press_ctrl_v();
        // 첨부는 KakaoTalk이 파일을 읽어 미리보기를 만드는 시간이 필요해 넉넉히 대기.
        sleep(Duration::from_millis(1200));
        press_enter();
        sleep(Duration::from_millis(600));
    });
    dbg_send(|| {
        format!(
            "첨부 전송 완료: {}",
            paths
                .map(|p| format!("{}개", p.len()))
                .unwrap_or_else(|| "클립보드".into())
        )
    });
    Ok(())
}

/// 파일 경로들을 CF_HDROP 형식으로 클립보드에 올린다(Explorer 복사와 동일).
fn set_clipboard_files(paths: &[PathBuf]) -> Result<(), PlatformError> {
    // 절대 경로 UTF-16(각각 널 종료) + 마지막 추가 널.
    let mut list: Vec<u16> = Vec::new();
    for p in paths {
        let abs = std::fs::canonicalize(p).map_err(|e| {
            PlatformError::Other(format!("파일 경로 확인 실패 {}: {}", p.display(), e))
        })?;
        // canonicalize의 \\?\ 접두사는 CF_HDROP에서 제거한다.
        let s = abs.to_string_lossy();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        list.extend(s.encode_utf16());
        list.push(0);
    }
    list.push(0); // double-null terminator

    let header = size_of::<DROPFILES>();
    let total = header + list.len() * 2;

    unsafe {
        let hglobal: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, total)
            .map_err(|e| PlatformError::Other(format!("GlobalAlloc 실패: {e}")))?;
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            return Err(PlatformError::Other("GlobalLock 실패".into()));
        }
        // DROPFILES 헤더.
        let df = ptr as *mut DROPFILES;
        (*df).pFiles = header as u32;
        (*df).pt = POINT { x: 0, y: 0 };
        (*df).fNC = BOOL(0);
        (*df).fWide = BOOL(1); // UTF-16 경로.
                               // 파일 목록 복사.
        let dst = (ptr as *mut u8).add(header) as *mut u16;
        std::ptr::copy_nonoverlapping(list.as_ptr(), dst, list.len());
        let _ = GlobalUnlock(hglobal);

        // 클립보드 열기(점유 중일 수 있어 몇 번 재시도).
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            sleep(Duration::from_millis(50));
        }
        if !opened {
            // 드문 실패 경로: 작은 할당은 그대로 둔다(프로세스 종료 시 회수).
            return Err(PlatformError::Other("클립보드를 열지 못했습니다".into()));
        }
        let _ = EmptyClipboard();
        if SetClipboardData(CF_HDROP, Some(HANDLE(hglobal.0))).is_err() {
            let _ = CloseClipboard();
            return Err(PlatformError::Other(
                "클립보드에 파일을 올리지 못했습니다".into(),
            ));
        }
        // 성공 시 소유권이 시스템으로 넘어가므로 GlobalFree 하지 않는다.
        let _ = CloseClipboard();
    }
    Ok(())
}

/// Ctrl+V(붙여넣기)를 저수준 입력으로 보낸다.
fn press_ctrl_v() {
    const V: u16 = 0x56;
    let seq = [
        vk_input(VK_CONTROL.0, false),
        vk_input(V, false),
        vk_input(V, true),
        vk_input(VK_CONTROL.0, true),
    ];
    unsafe {
        SendInput(&seq, size_of::<INPUT>() as i32);
    }
}

fn vk_input(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
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
