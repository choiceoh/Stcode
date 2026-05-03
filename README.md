# Stcode

코드를 모르는 바이브코더를 위한 macOS 데스크톱 코딩 에이전트.

- **GUI**: Zed의 [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) main HEAD (Apache-2.0)
- **백엔드**: [OpenAI Codex CLI](https://github.com/openai/codex)의 fork (Apache-2.0) + `app-server` JSON-RPC
- **LLM**: 로컬 vLLM (예: qwen3.6-35b-a3b) — fork patch가 vLLM 호환 처리
- **타깃**: macOS 우선 — 사내/팀 도구

## 사전 요구사항

- macOS 13+
- Xcode + Metal Toolchain (`xcodebuild -downloadComponent MetalToolchain`)
- Rust stable (`brew install rustup` → `rustup default stable`)
- vLLM 서버 (OpenAI Responses API 지원, `/v1/responses` 엔드포인트). 모델은 reasoning model (qwen3.6-a3b 등) 가능.

## 빌드 & 실행

### 1) codex fork 클론 + 빌드 (한 번)

```bash
cd ~/Documents/GitHub
git clone --depth 1 https://github.com/openai/codex.git codex-fork
# Stcode patch 적용 후 빌드 (이 repo의 patch는 별도)
cd codex-fork/codex-rs && cargo build -p codex-cli
```

Stcode가 `~/Documents/GitHub/codex-fork/codex-rs/target/debug/codex`를 자동으로 사용. 다른 위치면 `STCODE_CODEX_BIN=/path/to/codex` 환경변수.

### 2) Stcode 빌드 + 실행

```bash
cd ~/Documents/GitHub/Stcode
cargo run -p stcode-app
```

### 3) 사용

1. **📁 폴더 열기** → 프로젝트 폴더 선택
2. 인풋바에 한국어 prompt 타이핑 → ↵ Enter (또는 ↵ 보내기 버튼)
3. 응답이 채팅에 흘러나옴
4. 메시지 텍스트는 마우스 드래그로 selection + ⌘C 복사

### 환경변수

- `STCODE_CODEX_BIN` — codex 바이너리 절대 경로 override
- `STCODE_VLLM_COMPAT=1` — fork patch 활성 (Stcode bridge.rs가 자동 set)
- `RUST_LOG=info,stcode=debug` — 더 자세한 로그

## .app 번들 빌드 (사내 배포용)

```bash
bash scripts/build-app.sh                 # release + ad-hoc codesign
bash scripts/build-app.sh --debug         # debug 빌드 (디버깅용)
bash scripts/build-app.sh --no-codesign   # CI 등
```

결과: `dist/Stcode.app`. 더블클릭으로 실행. **첫 실행 시** Gatekeeper 경고가
뜨면 Finder 우클릭 → 열기 → "열기" 한 번만. 그 다음부턴 더블클릭 그대로.

배포: `ditto -c -k --keepParent dist/Stcode.app dist/Stcode.app.zip` 으로 압축
해서 사내 공유. Apple Developer 계정으로 notarization 까지 하려면 별도 작업
(v1엔 ad-hoc 만 — 사내라 충분).

번들엔 codex 바이너리는 **포함하지 않음**. 사용자가 fork 빌드 또는 brew로
설치한 codex를 자동 탐지 (`STCODE_CODEX_BIN` ENV / `~/Documents/GitHub/codex-fork`
경로 / `/opt/homebrew/bin` 순).

## 워크스페이스 구조

```
crates/
  stcode-app/         GPUI 앱 (Welcome / Chat / SelectableText / ChatInput)
  stcode-codex/       codex app-server JSON-RPC 클라이언트 + ThreadSession
  stcode-vibe/        바이브코더 안전 레이어 (M3 예정)
  stcode-vllm-proxy/  HTTP 프록시 (fork patch 도입 후 사실상 deprecated)
```

`docs/m1-wireframe.md` — UI 와이어프레임  
`docs/vllm-developer-role-fix.md` — vLLM chat template 가이드 (fork patch 도입 전 우회법)

## 코덱스 fork 패치 위치

`~/Documents/GitHub/codex-fork`에서:

- `codex-rs/codex-api/src/endpoint/responses.rs::stream_request` — outbound:
  - input array의 `type=message` 외 항목 drop (reasoning, function_call 등)
  - content array → string concat
  - `developer` role → `system` (qwen 등 호환)
  - system 메시지를 맨 앞으로 sort
  - top-level `instructions` 필드 제거 (input[0]에 중복)

게이팅: ENV `STCODE_VLLM_COMPAT=1`. OpenAI 본가 트래픽 영향 없음.

## 마일스톤

- **M0** ✅ scaffolding — GPUI 윈도우 + codex initialize 핸드셰이크
- **M1** ✅ 채팅 PoC — 폴더 선택, 한국어 입력, multi-line wrap, 응답 스트리밍, vLLM 호환 (fork)
- **M2** 승인 다이얼로그, command 출력, 파일 변경 알림
- **M3** 바이브 안전 레이어 — auto-git, 되돌리기, 친화적 에러
- **M4** 팀 배포 — `.app` 번들, 사내 코드사이닝

자세한 계획은 `~/.claude/plans/zed-gui-typed-trinket.md`.

## 라이선스 주의

- `gpui`, `codex` 모두 Apache-2.0
- **Zed 본체의 gpui 외 crate (editor, ui, workspace 등)는 GPL/AGPL** — 코드 한 줄도 복사 금지
- Stcode 자체는 사내 도구 (private)
