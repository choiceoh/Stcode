//! 바이브코더 안전 레이어.
//!
//! 모듈:
//! - [`git_safety`]: turn 단위 자동 git commit + 되돌리기. v1 핵심.
//! - [`friendly`]: codex/git/network 기술 메시지 → 한국어 친화 메시지 변환.
//! - [`settings`]: model/provider 등 사용자 설정 — Application Support toml.
//! - (예정) `keychain`: macOS Keychain에 API 키 저장/로드.

pub mod friendly;
pub mod git_safety;
pub mod settings;

pub use friendly::translate as friendly_translate;
pub use git_safety::{
    auto_commit_turn, current_head, ensure_repo, revert_to, GitError, TurnCommit,
};
pub use settings::Settings;
