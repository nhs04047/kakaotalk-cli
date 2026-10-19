---
name: kakaocli
description: 로컬 PC 카카오톡을 CLI/MCP로 조회·검색·통계·전송·감시한다. 사용자 본인 PC·본인 계정 전용. 카카오톡이 실행 중이어야 한다.
---

# kakaocli

PC 카카오톡을 **에이전트가 다루게** 하는 도구. 로컬 암호 DB(SQLCipher .edb)를 읽어 조회·검색·통계를 내고, 접근성/UI 자동화로 전송한다. 네트워크 프로토콜(LOCO)은 건드리지 않는다.

## 전제 조건

- **카카오톡이 실행 중**이어야 한다. (Windows는 복호화 키가 실행 중 프로세스 메모리에만 있음)
- **Windows**: 특정 방을 읽거나 감시·집계하려면 그 방을 앱에서 **한 번 열어** 둬야 한다(방별 키가 상주). 그래서 `stats`/`watch`는 "연 방"만 커버할 수 있고, 결과의 `coverage.partial`로 부분 집계 여부를 알린다.
- **macOS**: 단일 DB라 전 방 조회·집계 가능. 전송·조회는 접근성(AX) 권한이 필요하다.

## 사용 방법 (둘 중 하나)

### A. MCP 서버 (권장, 에이전트용)

`kakaocli-mcp` 바이너리를 MCP stdio 서버로 등록한다. 도구:

| 도구 | 설명 | 주요 인자 |
|------|------|-----------|
| `chats` | 방 목록 | `limit` |
| `messages` | 방 메시지 | `chat`(부분일치) 또는 `me`, `since`, `limit` |
| `find` | 검색 | `keyword`, `rooms`, `friends`, `all` |
| `stats` | 통계(방별·날짜별·시간대별) | `since`, `top` |
| `check` | 상태 점검 | — |
| `send` | 전송 | `chat` 또는 `me`, `message`, `confirm` |

**전송 안전장치**: `send`는 `confirm: true`일 때만 실제 전송한다. 기본값은 미리보기(dry-run). 실제 전송 전에 반드시 사용자 의도를 확인하라.

### B. CLI 직접 호출

모든 읽기 명령은 `--json`을 지원한다(구조화 출력).

```bash
kakaocli check --json                 # 상태 점검 (별칭: doctor)
kakaocli chats --json                 # 방 목록
kakaocli msg --chat "짱정하" --json    # 방 메시지 (또는 --me)
kakaocli find "회의" --json            # 검색 (--rooms/--friends/--all)
kakaocli stats --since 30d --json     # 통계
kakaocli watch --follow               # 새 메시지 감시 (별칭: sync/tail)
kakaocli send "짱정하" "안녕" --dry-run # 미리보기 (실제 전송은 --yes)
kakaocli send --me --file a.png --file b.pdf -y   # 파일·이미지 전송(최대 30개)
kakaocli send "짱정하" --clipboard -y             # 현재 클립보드의 이미지·파일 전송
```

> 이미지 전송(`--file`/`--clipboard`)은 현재 **Windows 전용**이며, 텍스트 전송과 마찬가지로 **대상 채팅창이 열려 있어야** 합니다(열린 창에 이미지 비트맵 붙여넣기 방식). **이미지 파일만 지원**합니다(png/jpg/jpeg/gif/bmp/webp) — 카카오톡 입력창이 이미지 붙여넣기만 받기 때문이며, 일반 문서 파일 전송은 아직 미지원입니다. 첨부와 텍스트 메시지는 함께 보낼 수 없습니다.

## 전송 규칙 (중요)

- 기본적으로 **확인을 거친다**. 비대화형(에이전트)은 `--yes`(CLI) 또는 `confirm:true`(MCP)로 진행한다.
- 전송 직전 **대상 방 제목을 재검증**한다. 단, **동명(同名) 방은 제목만으로 구분되지 않으므로** 오송신을 완전히 막지는 못한다. 중요한 전송은 사용자에게 방을 확인받아라.
- `watch --webhook <URL>`의 외부 전송은 **기본 비활성**이며, 쓸 때 대상·전송 데이터를 표시한다.

## 실패 모드

- "DEK를 확인할 수 없습니다" → 카카오톡이 꺼져 있거나(켜라), 해당 방을 아직 안 열었다(Windows: 앱에서 방 열기).
- "DB를 열 수 없습니다"/"스키마 불일치" → 카카오톡 업데이트로 깨졌을 수 있다. `check`로 버전·상태를 확인하라.
- 실패는 조용히 성공으로 둔갑하지 않는다. 비정상 종료 코드와 메시지를 신뢰하라.

## 범위 밖

- 타인 기기·타인 계정, 대량 발송, 봇 서비스, 프로토콜 모사. 자기 계정·자기 PC 전용이며 서비스 약관 리스크는 사용자 책임이다.
