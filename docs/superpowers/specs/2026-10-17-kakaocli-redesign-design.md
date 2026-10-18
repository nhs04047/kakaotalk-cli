# kakaocli 재설계 — 설계 문서

작성일: 2026-10-17
상태: 검토 반영본 (승인 대기)

## 1. 배경과 의도

kakaocli는 PC 카카오톡을 사람이 아니라 **AI 에이전트가 다루게** 하려는 도구다. 사용자가 자기 PC·자기 계정의 카카오톡 데이터를 CLI와 구조화된(JSON) 출력, 그리고 **MCP 서버**로 조회·전송·관찰하도록 만든다. CLI-Anything 철학(모든 소프트웨어를 에이전트가 쓰기 좋게)을 따르되, 카카오톡은 소스도 공개 API도 없는 폐쇄 앱이라 그 방식을 그대로 쓸 수 없다.

### REA 분석이 확정한 사실 (2026-10-17, Ghidra 12.1.4)

KakaoTalk.exe(Windows, 26.8.2.5324, 32비트)는 **Themida 패커로 보호**돼 있다. `.themida`(14.5MB)·`.boot`(8.9MB)·`.vm_sec` 섹션에 실제 코드가 암호화·가상화돼 있고, 실행 시점 메모리에서만 복호화된다. Ghidra는 함수 34개·문자열 115개(언패킹 스텁 잔여물)만 복원했다. 노출 import는 WS2_32(소켓), WINHTTP, ADVAPI32!CryptHashData, WINTRUST(자기무결성/안티변조), OLEACC!AccessibleObjectFromWindow(접근성), IMM32, Vox3.dll뿐이다.

**결론**: 자체 네트워크 프로토콜(LOCO)·암호 로직을 **정적 분석으로 추출하는 길은 닫혀 있다.** 동적 언패킹은 안티디버그·안티변조가 걸려 있고 업데이트마다 깨지며 계정 제재 위험이 가장 크다. 따라서 kakaocli는 LOCO를 건드리지 않는다.

### 두 개의 안정 통로 위에 설계한다

1. **로컬 암호 DB(.edb, SQLCipher) 읽기** — 조회·검색·통계·알림감시. DEK는 실행 중 프로세스 메모리에서 추출(실증됨).
2. **접근성/UI 자동화** — 전송과 실시간 동작. REA가 OLEACC/UIA 노출을 확인.

## 2. 목표 (Goals)

- 사용자 자신의 PC·계정에 대해 CLI로: **조회·검색 / 통계 / 전송 / 알림감시(watch)**.
- 모든 읽기 명령은 사람용 출력과 **`--json`**을 함께 제공. JSON은 안정 스키마로 고정·문서화.
- **MCP 서버** 제공 — 위 기능을 MCP 도구로 노출해 Claude 등 에이전트가 CLI를 셸 호출하지 않고 직접 쓰게 한다.
- **Windows + macOS** 둘 다.
- 에이전트 자가발견용 `--help`와 **SKILL.md**.
- 카카오톡 **업데이트에 강인** — 스키마·UI 변화는 조용한 실패가 아니라 명확한 에러로.

## 3. 비목표 (Non-goals)

- LOCO 등 네트워크 프로토콜 구현/모사.
- KakaoTalk.exe 패치·후킹·메모리 쓰기·안티변조 우회.
- 서버 직결 봇/자동응답, 대량 발송, 다계정 운영.
- 카카오톡 대체 GUI.
- 타인 기기·타인 계정 대상 동작.

## 4. 안티목표 (Anti-goals) — 되더라도 실패로 간주

- 계정 제재·보안 경고를 유발하는 동작을 기본 경로에 두는 것.
- 조용히 틀린 데이터(복호화 실패를 성공처럼, 잘못된 방에 전송). **실패는 반드시 시끄럽게.**
- 사용자 모르게 메시지를 보내거나 데이터를 외부로 반출하는 것.

## 4.5 리스크 (명시)

- **약관**: 자기 계정이라도 UI 자동화·로컬 DB 접근은 카카오 서비스 약관 위반 소지가 있고 계정 조치 위험이 실재한다. 사용자는 이를 감수하고 자기 책임으로 사용한다. 기본값은 항상 가장 보수적인 쪽.
- **버전 취약성**: DEK 메모리 스캔은 카카오톡 버전에 민감하다. 업데이트 시 스캔이 빗나가면 **실패로 드러내야** 하며(조용한 오독 금지), `doctor`가 선제 점검한다.

## 5. 제약 (Constraints)

- **Windows**: DEK가 실행 중 프로세스 메모리에만 존재 → 읽기 명령도 카카오톡이 켜져 있어야 한다. 특정 방 chatLogs는 **그 방을 앱에서 한 번 연 뒤라야** 읽을 수 있다(방별 .edb + 방별 DEK 상주). 바이너리는 Themida + WINTRUST 자기검증 → 프로세스는 **읽기만**, 절대 쓰지 않는다.
- **macOS**: 단일 DB. 키는 userId+device UUID 기반 PBKDF2 파생. 전송·조회는 접근성(AX) 경유.
- DB는 항상 `SQLITE_OPEN_READ_ONLY`.
- 자격증명(키 등)은 OS 키체인에만. 코드·로그·출력 평문 노출 금지.
- 전사 보안정책: 시크릿 하드코딩 금지, `.env`/`*.key`/`*.pem` 커밋 금지, 외부 통신은 대상·데이터 명시.
- 단일 사용자·단일 로컬 머신. 동시성·멀티테넌시는 범위 밖.

### 5.1 플랫폼 비대칭 (중요)

통계·알림감시는 **전 방 커버리지가 플랫폼마다 다르다.**
- **macOS**: 단일 DB라 전 방 통계·감시 가능.
- **Windows**: DEK가 상주한 방(= 앱에서 연 방)만 읽힌다. "전 방 통계"는 원천적으로 불가. 열린 방 집합으로 한정되며, 명령은 **커버리지(대상 방 수/전체 방 수)를 결과에 표시**해 사용자가 부분 집계임을 알게 한다.

## 5.5 현재 구현 현황 (본진 = project 레포 기준)

본진 `project/kakaotalk-cli`(cwd)의 실제 상태(2026-10-17 확인):

- **구현된 명령**: `check`, `auth`, `chats`, `msg`(--me/--since/--after), `find`(exact/regex/rooms/friends/all), `query`(raw SQL), `send`(--dry-run/--yes), `inspect`, `sync`(--follow/--interval/--chat/--webhook/--since), `login`. 전역 `--json`.
- **플랫폼 통로**: Windows DEK 스캔(`dek.rs`/`windows.rs`), SQLCipher 읽기전용 리더(Win/mac 이중 스키마), Windows UIA 전송(`winsend.rs`/`winuia.rs`), macOS AX 전송(단일 `darwin.rs`), 키체인 저장(`auth`).
- **없는 것(본진 기준)**: `members` 명령, 구조화 JSON(`json_out`/`rich`), 파일·클립보드 첨부 전송, 분리된 mac 모듈.

### 별도 작업 사본에서 이식할 것

더 진전된 별도 작업 사본에 있는, 본진에 없는 자산:
- `members` 명령(단톡방 멤버 조회).
- `json_out.rs` + `rich.rs`: 메시지에 구조화 카드(`rich`)를 덧붙이는 JSON — **에이전트/MCP에 특히 중요**.
- 첨부 전송(`send --file`/`--clipboard`, `send_attach`), `attach_match`/`send_match`/`chat_select` 헬퍼.
- 분리된 macOS 모듈(`mac/` : ax/send/focus/window/clipboard) — 단일 `darwin.rs`보다 유지보수 쉬움.

### 신규로 만들 것
1. **`stats`** — 방별·기간별·시간대별 메시지 집계(Windows는 §5.1 커버리지 한정). 축·SQL은 빌더.
2. **MCP 서버**(§6.6) — 새 크레이트.
3. **SKILL.md** — 에이전트용 사용법.
4. **JSON 스키마 문서화·고정**.
5. **webhook 게이팅·send 확인 모델**(§6.5 결정 반영).
6. 명칭 정리(check↔doctor, sync↔watch 별칭).

**검증 부채**: Windows는 테스트 경로 존재, **macOS 실기기 send/watch/stats 검증 미완**. 구현이 아니라 검증·리스크 항목.

## 6. 결정된 설계 (확정)

사용자가 정한 것:
- **언어: Rust**, 단일 바이너리.
- **플랫폼: Windows + macOS.**
- **v1 기능: 조회·검색 / 통계 / 전송 / 알림감시(watch) + MCP 서버.**
- **기반: `project/kakaotalk-cli`(본진)**, 별도 작업 사본의 mac·members·구조화 JSON·첨부전송을 이식.

제안 구조(검토 대상):
- **워크스페이스**: `core`(모델·kdf·경로), `db`(SQLCipher 읽기전용·스키마 추상화), `platform`(Win: DEK·UIA / mac: 키파생·AX), `auth`(키체인), `cli`(clap·표+`--json`), **신규 `mcp`(MCP 서버)**.
- **명령 그룹**: `chats` `msg` `members` `find` `stats` `send` `watch`(=sync) `login`/`auth` `doctor`(=check).
- **출력 계약**: 모든 읽기 명령 `--json`, 안정 키로 고정·문서화. 에러도 JSON(코드·메시지).
- **전송 안전장치**: 전송 직전 대상 방 제목 재확인 + `--dry-run` + 기본 확인. **한계: 동명(同名) 방은 제목만으로 구분 불가** → 오송신을 "완화"하되 "차단"이라 주장하지 않는다. 가능하면 방 식별자(logId/roomId) 기반 지정도 지원.
- **실패 가시화**: DEK 미획득·DB 잠금·스키마 불일치·대상 방 미개방을 구분된 에러로. `doctor` 선제 점검.
- **버전 드리프트**: DB 스키마·카카오톡 버전 감지, 불일치 시 경고 + DEK 스캔 자가검증.

## 6.5 저자 결정 (확정, 2026-10-17)

1. **webhook/외부 전송**: 기본 off + 명시 동의(대상·전송 데이터 표시). §3·§4·보안정책 정합.
2. **에이전트용 `send` 확인**: 기본 확인 on, `--yes`로 생략. 전송 직전 방 제목 재검증은 항상.
3. **기반 레포**: `project/kakaotalk-cli` 본진, 별도 작업 사본 자산 이식.

## 6.6 MCP 서버 설계 (구현 반영)

- **형태**: 새 크레이트 `kakaocli-mcp`, **stdio** MCP 서버(newline-delimited JSON-RPC 2.0, 프로토콜 `2024-11-05`). 의존성은 serde_json뿐, 비동기 런타임 없음.
- **구현 방식(v1 결정)**: 각 도구 호출을 **같은 폴더의 `kakaocli` 바이너리에 `--json`으로 위임**한다. Windows DEK 스캔·macOS AX·스키마 분기 등 모든 플랫폼 로직을 단일 진실 원천으로 재사용하기 위한 의도적 단순화. 인프로세스 라이브러리화는 호출 지연·배치 전송이 병목이 될 때의 후속 과제.
- **도구**: `chats`, `messages`(chat 또는 me), `find`, `stats`, `check`, `send`.
- **전송 도구 안전**: `send`는 `confirm:true`일 때만 실제 전송(`--yes`, 비대화형). 그 외에는 미리보기(`--dry-run`). 대상(`chat`/`me`)·본문을 명시 파라미터로 강제. 외부 webhook은 도구로 노출하지 않음.
- **출력**: CLI `--json`과 동일 스키마 재사용.
- **배포**: `docs/mcp.md`에 `.mcp.json` 등록 예시. 빌드 시 `kakaocli`와 `kakaocli-mcp`가 같은 `target/release/`에 생성(서버가 형제 경로에서 CLI를 찾음).

## 7. 빌더에게 남기는 것 (열린 결정)

- 이식 자산 순서·범위(최소: members, json_out/rich, mac 모듈; 첨부전송은 후순위 가능).
- `watch` 폴링 주기·WAL 관찰·diff 산출.
- `stats` 집계 축 목록과 SQL.
- JSON 스키마 정확한 필드(현 serde 모델 기반).
- mac/win 스키마 공통 추상화(트레잇) 경계.
- MCP 크레이트의 라이브러리 선택과 도구 스키마.
- 테스트 전략: 로컬 DB 픽스처 단위 테스트 + 실기기 E2E 범위.

## 8. 성공 기준

- Windows·macOS에서 `chats`/`msg`/`members`/`find`/`stats`/`send`/`watch`가 실제 데이터로 동작(Windows stats·watch는 §5.1 커버리지 한정).
- MCP 서버가 Claude 등에서 도구로 보이고, 읽기·전송·stats가 동작.
- 모든 읽기 명령·MCP 도구가 유효한 `--json`/구조화 출력을 내고 문서 스키마와 일치.
- 전송이 방 제목 재검증을 거치고 `--dry-run`이 아무것도 보내지 않음. 동명 방 한계가 문서·에러에 드러남.
- 복호화·전송·DEK 실패가 조용히 성공으로 둔갑하지 않음.
- `doctor`가 환경·버전 문제를 사람이 읽게 설명.
- SKILL.md만 보고 에이전트가 기본 작업 수행 가능.
- (검증) macOS 실기기에서 send/watch/stats 1회 이상 확인.
