use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpawnError {
    #[error("`codex` 바이너리를 찾을 수 없습니다. `brew install codex` 후 다시 실행해 주세요.")]
    NotFound,
    #[error("`codex` 실행 권한 확인 실패: {0}")]
    Permission(#[source] std::io::Error),
}

pub struct CodexBinary {
    pub path: PathBuf,
}

/// PATH에서 `codex`를 찾는다. macOS Homebrew 일반 경로도 보조 탐색.
pub fn find_codex_binary() -> Result<CodexBinary, SpawnError> {
    if let Some(p) = which("codex") {
        return Ok(CodexBinary { path: p });
    }
    for fallback in [
        "/opt/homebrew/bin/codex",
        "/usr/local/bin/codex",
        "/usr/bin/codex",
    ] {
        let p = PathBuf::from(fallback);
        if p.is_file() {
            return Ok(CodexBinary { path: p });
        }
    }
    Err(SpawnError::NotFound)
}

fn which(cmd: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(cmd);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
