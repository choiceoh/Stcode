//! 바이브코더 안전 레이어.
//!
//! 모듈:
//! - [`git_safety`]: turn 단위 자동 git commit + 되돌리기. v1 핵심.
//! - (예정) `friendly`: codex 기술 메시지 → 한국어 사용자 메시지 변환
//! - (예정) `keychain`: macOS Keychain에 API 키 저장/로드

pub mod git_safety;

pub use git_safety::{
    auto_commit_turn, current_head, ensure_repo, revert_to, GitError, TurnCommit,
};
