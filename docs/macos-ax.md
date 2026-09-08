# macOS 백엔드: objc2 → AXUIElement

## 개요

macOS에서 메시지 전송은 Accessibility API(AXUIElement)를 통해 KakaoTalk Mac 클라이언트를 자동화.

## 주요 참고: kakaocli의 CLAUDE.md

> kakaocli(Swift)의 검증된 트릭을 그대로 계승:
> - Self-chat은 `"badge me"` AX image descriptor로 식별 (이름 아님)
> - 메인 윈도우: `id="Main Window"` (로그인/로그인 둘 다)
> - 로그인 vs 로그인: 윈도우 타이틀("Log in" vs "KakaoTalk")으로 구분
> - AXTextField가 password 필드 (AXSecureTextField 아님)
> - kAXWindowsAttribute가 AXApplication을 반환할 수 있음 (히든 상태)
> - 채팅방 열기: AX row selection + Enter (double-click은 오프스크린 좌표 문제)
> - Paywall: 500x500 blank window, Escape로 닫기

## Rust objc2 바인딩 구조

```rust
use objc2::*;
use objc2_foundation::*;
use objc2_app_kit::*;

// KakaoTalk 앱 실행
fn launch_kakao() -> Result<NSRunningApplication> {
    let workspace = NSWorkspace::sharedWorkspace();
    let url = NSURL::fileURLWithPath("/Applications/KakaoTalk.app");
    let config = NSWorkspaceOpenConfiguration::new();
    let app = workspace.openApplicationAtURL(url, config)?;
    Ok(app)
}

// AX 요소 찾기
fn find_ax_element(pid: i32, attribute: &str, value: &str) -> Result<AXUIElement> {
    let app = AXUIElementCreateApplication(pid);
    // ... AXUIElementCopyAttributeValue 순회
}

// 채팅방 선택
fn select_chat(app_element: &AXUIElement, name: &str) -> Result<()> {
    // 1. kAXChildrenAttribute → 채팅방 리스트 획득
    // 2. 각 리스트 아이템의 kAXDescriptionAttribute 확인
    // 3. name과 일치하는 아이템 찾기
    // 4. kAXSelectedRowsAttribute로 선택 + Enter 전송
}

// 메시지 입력 및 전송
fn type_and_send(text_field: &AXUIElement, text: &str) -> Result<()> {
    // 1. kAXFocusedAttribute = true
    // 2. kAXValueAttribute에 텍스트 설정
    // 3. Enter 키 (CGEvent) 전송
}
```

## 전체 전송 플로우

```
1. KakaoTalk 실행 확인 (NSWorkspace)
   - 미실행 → launch / 로그인 필요 → auto-login
   - 실행 중 → activate

2. 메인 윈도우 찾기
   - id="Main Window" (주의: kAXWindowsAttribute가 AXApplication 반환 가능)
   - 상태표시줄 메뉴로 로그인 상태 확인

3. 채팅방 선택
   - 채팅 리스트 스크롤 영역 찾기
   - 각 AX리스트 아이템 순회 → kAXDescription으로 이름 매칭
   - kAXSelectedRowsAttribute(row_index) + Enter

4. 메시지 입력
   - 텍스트 필드에 kAXValueAttribute 설정
   - 또는 CGEvent 키 입력 시뮬레이션

5. 전송
   - Enter 키 전송
   - 전송 버튼 찾기 → kAXPressAttribute

6. 정리
   - 채팅방 닫기 (Command+W)
```

## 로그인 자동화

```rust
fn auto_login() -> Result<()> {
    // 1. 상태 확인 (status bar menu item에서 "Log out" 검색)
    // 2. 로그인 화면이면:
    //    a. email 필드 찾기 (AXTextField × 2)
    //    b. 첫 번째에 email 입력
    //    c. 두 번째에 password 입력
    //    d. "Log in" 버튼 클릭
    // 3. 로그인 완료까지 폴링 (status bar)
}
```

## 주의사항

- **비표준 AX 계층:** KakaoTalk의 AX 트리는 표준 macOS 가이드라인을 따르지 않음
- **윈도우 히든 상태:** 앱이 메뉴바에만 있을 때 AXWindow가 0개
- **오프스크린 좌표:** 스크롤된 채팅방의 AX 좌표가 화면 밖 → AX selection 사용
- **딜레이:** 각 액션 사이에 0.5~1초 대기 필수
- **디버깅:** `kakaocli inspect` 명령어로 AX 트리 덤프