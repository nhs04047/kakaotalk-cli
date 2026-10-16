# kakaocli MCP 서버

`kakaocli-mcp`는 kakaocli 기능을 **MCP 도구**로 노출하는 stdio 서버다. Claude 등 에이전트가 CLI를 직접 셸 호출하지 않고 도구로 쓸 수 있다.

## 동작 방식

MCP 서버는 각 도구 호출을 **같은 디렉터리의 `kakaocli` 바이너리**로 `--json`과 함께 위임한다. 덕분에 Windows DEK 스캔·macOS AX·스키마 분기 등 모든 플랫폼 로직이 **단일 진실 원천**으로 재사용된다. (v1의 의도적 단순화 — 인프로세스 라이브러리화는 후속 과제.)

전송 라이브러리 경로가 필요해지는 조건: 호출 지연·배치 전송이 병목이 될 때.

## 등록

`kakaocli`와 `kakaocli-mcp` 바이너리가 **같은 폴더**에 있어야 한다(서버가 형제 경로에서 CLI를 찾는다). 빌드 산출물은 `target/release/`에 함께 생성된다.

### Claude Desktop / Claude Code (`.mcp.json` 또는 설정)

```json
{
  "mcpServers": {
    "kakaocli": {
      "command": "C:\\Users\\<you>\\...\\target\\release\\kakaocli-mcp.exe"
    }
  }
}
```

macOS는 `command`를 `.../target/release/kakaocli-mcp`로 지정한다.

## 도구

| 도구 | 설명 | 인자 |
|------|------|------|
| `chats` | 방 목록 | `limit?` |
| `messages` | 방 메시지 | `chat?` 또는 `me?`, `since?`, `limit?` |
| `find` | 검색 | `keyword`(필수), `rooms?`, `friends?`, `all?` |
| `stats` | 통계 | `since?`, `top?` |
| `check` | 상태 점검 | — |
| `send` | 전송 | `chat?` 또는 `me?`, `message`(필수), `confirm?` |

### 전송 안전

- `send`는 `confirm: true`일 때만 실제 전송(`--yes`, 비대화형). 그 외에는 미리보기(`--dry-run`).
- 전송 직전 대상 방 제목을 재검증하나, **동명 방은 구분 불가** → 중요한 전송은 사용자 확인 필요.
- webhook 등 외부 전송 도구는 노출하지 않는다.

## 프로토콜

newline-delimited JSON-RPC 2.0 over stdio. 지원 메서드: `initialize`, `tools/list`, `tools/call`, `ping`. 프로토콜 버전 `2024-11-05`.
