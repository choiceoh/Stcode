# Stcode

코드를 모르는 바이브코더를 위한 macOS 데스크톱 코딩 에이전트.

- **GUI**: Zed의 [GPUI](https://crates.io/crates/gpui) (Apache-2.0)
- **백엔드**: [OpenAI Codex CLI](https://github.com/openai/codex)의 `codex app-server` (Apache-2.0)
- **타깃**: macOS 우선 — 사내/팀 도구

## 사전 요구사항

- macOS 13+
- Rust stable (rustup 권장)
- `codex` CLI: `brew install codex` 또는 `npm i -g @openai/codex`
- OpenAI API 키 (앱 첫 실행 시 입력 — macOS Keychain에 저장)

## 빌드 & 실행 (M0)

```bash
cargo run -p stcode-app
```

성공 시:
1. GPUI 윈도우가 뜨고 "Stcode" 텍스트가 보임
2. 백그라운드 스레드에서 `codex app-server` spawn → `initialize` 핸드셰이크 시도
3. 콘솔(`RUST_LOG=info,stcode=debug`)에 `codex initialize OK: …` 또는 친화적 에러 출력

`codex`가 PATH에 없으면 친화적 에러 메시지가 콘솔에 노출됨 (정상).

## 워크스페이스 구조

```
crates/
  stcode-app/    GPUI 앱 진입점, 뷰
  stcode-codex/  codex app-server JSON-RPC 클라이언트
  stcode-vibe/   바이브코더 안전 레이어 (M3)
```

## 마일스톤

- **M0** ✅ scaffolding — GPUI 윈도우 + codex initialize 핸드셰이크
- **M1** 채팅 PoC — 폴더 선택, 메시지 스트리밍
- **M2** 승인 + 도구 — 모달 다이얼로그, command output
- **M3** 바이브 안전 레이어 — auto-git, 되돌리기, 친화적 메시지
- **M4** 팀 배포 — `.app` 번들, 사내 코드사이닝

자세한 계획은 `~/.claude/plans/zed-gui-typed-trinket.md`.

## 라이선스 주의

- `gpui`, `codex` 모두 Apache-2.0이라 사내 비공개 도구로 임베드 가능
- **Zed 본체(zed-industries/zed의 gpui 외 크레이트)는 GPL/AGPL** — 코드 단 한 줄도 복사 금지
