# macOS send / inspect 실기 검증 체크리스트

> 대상: `darwin.rs`의 `send_message` / `dump_ax_tree` (구현 완료, 실기 미검증)
> 목적: 이미 작성된 AX 자동화 코드가 실제 카카오톡에서 동작하는지 확인.
> 실패 시 각 단계의 "실패하면" 절차대로 로그를 남겨 오면 세션에서 수정.

## 사전 준비

- [ ] 릴리즈 빌드: `cargo build --release`
- [ ] 카카오톡 데스크톱 실행 + 로그인 상태
- [ ] **손끝 권한(Accessibility)**: 시스템 설정 → 개인정보 보호 및 보안 → 손쉬운 사용 → 터미널(또는 빌드된 바이너리) 허용
  - `kakaocli inspect` 최초 실행 시 권한 프롬프트가 뜨면 허용 후 재실행
- [ ] 테스트용 안전 대상 확보: **나와의 채팅**(self-chat)을 1순위로 사용 (오발송 방지)

## 1. inspect (읽기 전용 — 먼저, 리스크 없음)

- [ ] `kakaocli inspect --depth 3`
  - 기대: AX 트리가 출력됨 (window → 채팅 리스트 role → row들)
  - **실패하면**: 전체 출력 + `RUST_LOG=debug kakaocli inspect --depth 5` 재실행 로그 저장
- [ ] `kakaocli inspect --chat "나" ` (또는 self-chat 표시명)
  - 기대: 해당 방으로 필터된 서브트리
  - **실패하면**: `--chat` 없이 나온 전체 트리에서 self-chat row의 role/title 값을 확인해 공유
- [ ] `kakaocli --json inspect --depth 2`
  - 기대: 유효한 JSON (파이프 가능: `| jq .role`)

### inspect에서 확인할 핵심 값 (send가 이걸 의존함)
- [ ] 채팅 리스트 컨테이너의 **role** (코드: `get_chat_list_role`)
- [ ] 각 채팅 row의 **표시명이 title/description 중 어디**에 들어오는지 (`get_row_display_name`)
- [ ] 열린 대화창 상단 제목의 위치 (`verify_chat_window`가 여기서 재확인)

## 2. send — dry-run (전송 안 함)

- [ ] `kakaocli send --me "test" --dry-run`
  - 기대: `🔍 Dry-run: would send to '_': test` 만 출력, 실제 전송 없음
- [ ] `kakaocli send "친구이름" "test" --dry-run`
  - 기대: dry-run 메시지, 전송 없음

## 3. send — self-chat 실전송 (가장 안전)

- [ ] `kakaocli send --me "kakaocli 테스트 $(date)" -y`
  - 기대: 나와의 채팅에 메시지 도착, `✅ Message sent to '_'`
  - 검증 포인트(코드 경로):
    - [ ] self-chat row 탐색 성공 (`find_self_chat_row`)
    - [ ] 방 열림 + 키보드 선택 (`select_ax_row_via_keyboard`)
    - [ ] 제목 재확인 통과 (`verify_chat_window`) — 엉뚱한 방이면 여기서 중단돼야 정상
    - [ ] 텍스트 타이핑 (`type_text`) — **한글 입력 정상 여부** 중점 확인
    - [ ] 엔터 전송 (`press_enter`)
  - **실패하면**: 어느 단계 에러인지 메시지 저장 + `RUST_LOG=debug` 로그

## 4. send — 일반 채팅방 (이름 매칭 / 스크롤 / 검증)

> ⚠️ 실제 사람에게 감. 미리 양해된 대상 또는 본인 부계정 사용.

- [ ] `kakaocli send "정확한방이름" "hi" -y`
  - [ ] 화면에 안 보이는 방도 스크롤로 찾는지 (`find_chat_row_with_scroll` / `scroll_down_cg`)
- [ ] **오검증 테스트**: 존재하지 않는 이름 → `ChatVerificationFailed` 또는 못 찾음 에러로 **안전하게 실패**하는지 (엉뚱한 방 전송 0건이어야 함)
- [ ] **동명이인 테스트**: 같은 이름 방이 2개 이상일 때 `AmbiguousChatName` 뜨는지
- [ ] 인터랙티브: `kakaocli send` (인자 없이) → 방 선택 프롬프트 → 메시지 입력 → 확인 프롬프트

## 5. 확인 프롬프트 / 비대화형

- [ ] `-y` 없이 `kakaocli send --me "x"` → 확인 프롬프트에서 **기본값 No** 인지, `n` 누르면 취소되는지
- [ ] 파이프(비TTY): `echo | kakaocli send --me "x"` → `--yes 필요` 에러로 막히는지

## 결과 리포트 양식 (실패 시 이대로 붙여줘)

```
명령: <실행한 커맨드>
기대: <무엇이 되어야 했나>
실제: <출력 전체 / 에러 메시지>
로그: <RUST_LOG=debug 재실행 로그, 있으면>
inspect 트리: <관련되면 해당 서브트리>
```
