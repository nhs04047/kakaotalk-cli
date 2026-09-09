# sync 설계 (kakaocli sync)

- 작성일: 2026-09-09
- 상태: 승인됨 (구현 대기)
- 범위: `kakaocli sync` 커맨드 신규 구현. Phase 4 항목.
- 관련 문서: `docs/flows.md` §8, `docs/roadmap.md` Phase 4, `DESIGN.md`

## 1. 목적

카카오톡 로컬 DB를 폴링해 **새 메시지를 증분으로 스트리밍**한다. 푸시 알림 경로가 없으므로 폴링이 유일한 수단이다. 출력은 사람이 읽는 형태를 기본으로 하고, `--json`으로 NDJSON 스트림을, `--webhook`으로 HTTP POST 전달을 지원한다.

플랫폼 전제:
- macOS: 앱 실행 없이 DB만으로 동작한다.
- Windows: DEK 확보가 선행돼야 하므로 KakaoTalk.exe 실행이 필요하다 (본 스펙 범위 밖, Windows DEK 스캐너 완료 후 자동으로 열린다).

## 2. CLI 표면

```
kakaocli sync [--follow] [--interval <초>] [--chat <이름>] [--webhook <URL>] [--since <시간>] [--json]
```

| 플래그 | 기본값 | 의미 |
|--------|--------|------|
| `--follow` | false | 지속 폴링 스트리밍 |
| `--interval` | 2 | 폴링 간격(초). `--follow`일 때만 의미 있음 |
| `--chat` | 없음(전체 방) | 방 이름 부분일치. `resolve_chat_id`로 해석 |
| `--webhook` | 없음 | 새 메시지 배치를 이 URL로 POST |
| `--since` | 없음 | 최초 기준선을 이 시점으로 백필. `parse_since` 재사용 |
| `--json` | false | NDJSON 출력 (전역 플래그 `--json` 재사용) |

기존 `Command::Sync`는 `follow`/`interval`만 갖고 있으므로 `chat`/`webhook`/`since`를 추가한다.

## 3. 증분 모델

### 3.1 증분 키는 `logId`

`NTChatMessage.logId`를 기준으로 `logId > last`를 조회한다. `sentAt`을 쓰지 않는 이유:

- `sentAt`은 초 단위라 같은 초에 여러 건이 들어오면 경계에서 중복 또는 누락이 발생한다.
- 기기 시계 보정이나 서버 시각 차이로 단조성이 깨질 수 있다.
- `logId`는 삽입 순서로 증가하므로 `> last` 비교만으로 정확히 한 번씩 처리된다.

### 3.2 체크포인트

경로: `~/.kakaocli/sync.json` (`db_path`의 `user_id` 캐시와 동일한 디렉터리 규약)

형식:
```json
{ "global": 1500, "chat:123": 1490 }
```

- 전체 방 모드는 `global` 키, `--chat` 모드는 `chat:<chat_id>` 키를 쓴다. 두 모드가 서로의 진행 상태를 덮어쓰지 않는다.
- 배치를 성공적으로 출력할 때마다 저장한다. 중단(Ctrl-C, 크래시)돼도 다음 실행이 이어서 처리한다.
- 파일이 없거나 손상됐으면 빈 상태로 간주하고 새로 만든다 (에러로 중단하지 않는다).

### 3.3 최초 기준선

체크포인트에 해당 키가 없을 때:

- `--since`가 있으면: 그 시각 이후의 메시지를 백필 대상으로 삼는다. 구현상 `sentAt >= since`인 최소 `logId - 1`을 기준선으로 잡는다.
- `--since`가 없으면: 현재 `MAX(logId)`를 기준선으로 잡고 이번 실행에서는 0건을 출력한다. 첫 실행이 과거 전체를 쏟아내지 않게 한다.

### 3.4 실행 모드

- `--follow` 없음: 체크포인트 이후 새 메시지를 한 번 출력하고 체크포인트를 갱신한 뒤 종료한다. 외부 cron/스케줄러와 조합할 수 있다.
- `--follow`: 위 캐치업을 먼저 수행하고, 이후 `--interval`초마다 폴링을 반복한다. Ctrl-C로 종료한다.

## 4. 데이터 계층 (`kakaocli-db`)

신규 메서드 두 개를 추가한다.

```rust
/// logId 기준 증분 조회. chat_id가 None이면 전체 방.
pub fn messages_after(
    &self,
    after_log_id: i64,
    chat_id: Option<i64>,
    limit: u32,
) -> Result<Vec<Message>, DbError>;

/// 현재 최대 logId (기준선용). chat_id가 None이면 전체 방.
pub fn max_log_id(&self, chat_id: Option<i64>) -> Result<i64, DbError>;
```

`messages_after` SQL:
```sql
SELECT m.logId, m.chatId, m.authorId,
       COALESCE(u.displayName, u.friendNickName, u.nickName) AS senderName,
       m.message, m.type, m.sentAt
FROM NTChatMessage m
LEFT JOIN NTUser u ON m.authorId = u.userId AND u.linkId = 0
WHERE m.logId > ?
  AND (? IS NULL OR m.chatId = ?)
ORDER BY m.logId ASC
LIMIT ?
```

기존 `get_messages`와 달리 **`logId ASC`** 로 정렬한다. 스트림은 오래된 것부터 나가야 한다.

`limit`은 한 폴에서 가져올 상한이다(기본 500). 상한에 걸리면 체크포인트를 마지막 건으로 올린 뒤 같은 틱에서 한 번 더 당겨 밀린 분량을 따라잡는다.

방 이름 표시: 시작 시 `list_chats`로 `HashMap<i64, String>`(chat_id → 표시명)를 한 번 만들어 재사용한다. 맵에 없는 chat_id는 `chatId` 숫자를 그대로 표시한다. 새 방이 생기면 다음 실행에서 반영된다.

## 5. 출력

### 5.1 기본 (사람친화)

`display.rs`에 `print_sync_message`를 추가한다. 기존 색상/스타일 헬퍼를 재사용한다.

```
10:30 · [지수] 지수: 밥 먹었어?
10:31 · [팀 공지] 나: 확인했습니다
```

`--chat`으로 방이 고정된 경우 `[방]` 부분을 생략한다.

### 5.2 `--json` (NDJSON)

줄당 객체 하나. `sync --json | jq` 조합을 전제로 한다.

```json
{"type":"message","logId":1501,"chatId":123,"chat":"지수","sender":"지수","fromMe":false,"text":"밥 먹었어?","time":"2026-09-09T10:30:00+09:00","msgType":"text"}
```

| 필드 | 타입 | 비고 |
|------|------|------|
| `type` | string | 항상 `"message"`. 향후 이벤트 종류 확장 여지 |
| `logId` | number | 증분 키 |
| `chatId` | number | |
| `chat` | string | 방 표시명. 미상이면 chatId 문자열 |
| `sender` | string\|null | `senderName` |
| `fromMe` | bool | `is_from_me` |
| `text` | string\|null | 본문. 미디어 등은 null 가능 |
| `time` | string | RFC 3339 (로컬 오프셋) |
| `msgType` | string | `MessageType`의 소문자 이름 |

출력 즉시 flush 한다 (파이프에서 버퍼링으로 멈춰 보이지 않게).

## 6. `--webhook`

- 한 폴에서 얻은 새 메시지를 **JSON 배열**로 묶어 한 번 POST 한다 (건별이 아님).
- `Content-Type: application/json`, 타임아웃 5초, 실패 시 1회 재시도.
- 재시도까지 실패하면 stderr에 경고를 남기고 **스트림은 계속한다**. 체크포인트는 이미 출력한 지점으로 갱신한다(웹훅 실패로 stdout 스트림이 멈추지 않는다).
- 의존성: `reqwest` (blocking + rustls-tls, 기본 features 끔).

웹훅 페이로드 생성은 순수 함수로 분리해 단위테스트 대상으로 삼는다.

## 7. 에러 처리

| 상황 | 처리 |
|------|------|
| SQLite BUSY (카톡이 쓰는 중) | 연결에 `busy_timeout` 설정 + 폴 내 짧은 백오프 재시도. follow 루프를 죽이지 않는다 |
| DB 열기 실패 | 즉시 종료 (기존 `open_db` 에러 경로) |
| 체크포인트 파일 손상 | 경고 후 빈 상태로 재생성 |
| 웹훅 실패 | 경고 후 계속 (§6) |
| Ctrl-C | 루프 종료. 배치마다 체크포인트를 저장했으므로 유실 없음 |

## 8. 구조와 테스트 가능성

폴링 1회를 순수하게 분리한다.

```rust
struct SyncState { last_log_id: i64 }

/// DB에서 새 메시지를 당기고 state를 전진시킨다. 출력/웹훅은 하지 않는다.
fn poll_once(db: &Database, chat_id: Option<i64>, state: &mut SyncState, limit: u32)
    -> Result<Vec<Message>, DbError>;
```

`cmd_sync`는 `poll_once` → 출력 → 웹훅 → 체크포인트 저장 → 대기의 얇은 루프가 된다.

### 테스트 계획

- `messages_after`: 임시 SQLCipher DB에 행을 삽입해 (a) `logId ASC` 순서, (b) `> after` 경계(같은 logId 미포함), (c) `chat_id` 필터, (d) `limit` 상한을 검증한다. 기존 `kakaocli-db` 테스트 패턴을 따른다.
- `max_log_id`: 빈 테이블에서 0, 데이터 있을 때 최대값, `chat_id` 필터 동작.
- 체크포인트: 저장 → 로드 round-trip, 파일 없음, 손상된 JSON 세 경우.
- 웹훅 페이로드 빌더: 메시지 벡터 → JSON 배열 문자열 검증 (실제 POST는 수동 검증).
- `poll_once`: 새 메시지가 없을 때 빈 벡터 + state 불변, 있을 때 state가 마지막 logId로 전진.

## 9. 손대는 파일

| 파일 | 변경 |
|------|------|
| `kakaocli-db/src/lib.rs` | `messages_after`, `max_log_id` + 테스트 |
| `kakaocli-core/src/sync_state.rs` (신규) | 체크포인트 read/write + 테스트 |
| `kakaocli-core/src/lib.rs` | 모듈 등록 |
| `kakaocli-cli/src/main.rs` | `Command::Sync` 플래그 확장, `cmd_sync` 구현 |
| `kakaocli-cli/src/display.rs` | `print_sync_message` |
| `kakaocli-cli/Cargo.toml`, 워크스페이스 `Cargo.toml` | `reqwest` 추가 |
| `docs/flows.md` | §8 NDJSON 스키마를 본 스펙과 일치시킴 |
| `README.md` | 프로젝트 구조/커맨드 섹션 갱신 (사내 문서 정책) |
| `docs/roadmap.md` | Phase 4 sync 항목 체크 |

## 10. 명시적 비범위

- Windows에서의 동작 검증 (DEK 스캐너 선행 필요)
- 메시지 외 이벤트(읽음 표시, 방 생성/삭제) 스트리밍
- 웹훅 서명/인증 헤더
- 미디어 첨부 다운로드
