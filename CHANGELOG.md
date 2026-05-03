# Changelog

## v0.1.0 — 2026-05-03

첫 사내 배포. **바이브 코더용 macOS 자동 작업 에이전트 (Stcode)** 의 골든 패스가
끝까지 동작하는 시점.

### 핵심 워크플로우 (사용자 명시)
- **자동 모드**: 모든 권한 자동 통과 (`ApprovalPolicy::Never` + `SandboxMode::DangerFullAccess`).
  inbound approval request도 bridge 가 즉시 자동 Accept.
- **병렬 멀티 에이전트**: 사이드바에 세션 N개 동시 운영. 각 세션은 자체 tokio task로
  진짜 동시 polling. 한 세션 응답 중에 다른 세션 클릭으로 active 전환 가능.
- **사후 git 안전망**: 폴더에 `.git` 없으면 자동 init → turn 시작 직전 HEAD snapshot →
  종료 시 변화 있으면 자동 commit (`stcode: <prompt 첫 줄>`) → 헤더 "↶ 되돌리기" 칩
  으로 1클릭 hard reset.

### GUI
- GPUI (Zed main HEAD git rev pin) 단독. Welcome / Workspace 두 screen.
- 사이드바: 세션 list + status icon (`○ 시작 전 / ⏳ 작업 중 / ● 새 메시지 / ✓ 대기`)
  + "+ 새 세션" + "⚙ 설정" 버튼.
- 메인: 헤더 (status + revert 칩) / 메시지 / chips (model·자동모드·작업폴더) / 입력바.
- 채팅: Reasoning은 별도 회색 패널, 본 답변은 bubble. **Markdown 코드블록(fenced
  ```), heading(# ## ###), 리스트(- *)** 는 turn 종료 시 segment로 정리 (mono font
  코드블록, 큰 헤딩, bullet 리스트). inline code/bold 는 미지원 (다음).
- Tool Cards: commandExecution / fileChange / mcpToolCall / webSearch — 친화적
  요약 (raw 출력 노출 X).
- 설정 모달: provider/model 입력 → `~/Library/Application Support/Stcode/settings.toml`
  영구 저장. 새 세션부터 적용.

### 안전망 (`stcode-vibe`)
- `git_safety`: 자동 init / current_head / auto_commit_turn / revert_to.
- `friendly`: codex/git/network 에러 14패턴 → 한국어 + 액션 제안.
- `settings`: model/provider toml 영구 저장.

### 백엔드
- codex CLI fork (`STCODE_VLLM_COMPAT=1` 패치) + 로컬 vLLM (`/v1/responses`).
- ws 비활성화는 provider 이름이 "vllm" 포함 시 자동.

### 배포
- `bash scripts/build-app.sh` → `dist/Stcode.app` (ad-hoc codesign).
- 사내 zip 배포: `ditto -c -k --keepParent dist/Stcode.app dist/Stcode.app.zip`.

### 의식적 비목표 (v1)
- 파일 트리 / diff 뷰어 / 코드 에디터 패널 — IDE 아님.
- LSP / 터미널 패널 / 디버거 — 안 함.
- Windows / Linux — macOS only.
- 다중 사용자 / 플러그인 — 사내 범위 밖.
- codex 바이너리 자동 번들/업데이트 — 사용자가 fork/brew로 관리.
- API Keychain — vLLM dummy 키라 v1엔 dummy 그대로.
- inline markdown (`code`, **bold**) — line-level만 v1.

### 알려진 한계
- 한글 IME 일부 케이스에서 char boundary 이슈 가능 (utf16 변환으로 일반 케이스는 OK).
- codex 재spawn은 새 세션 시점에만 — 진행 중 세션은 model 변경 적용 안 됨.
- 한 세션이 무한 reasoning 빠지면 인터럽트 UI 없음 (codex turn/interrupt RPC는
  bridge 에 있지만 GUI 노출 안 됨).
