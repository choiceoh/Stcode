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

/// codex spawn 시 추가 옵션. `-c key=value` 인자로 codex 설정을 override 하고,
/// 자식 프로세스 환경변수도 명시 가능하다.
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// `(key, value)` 페어 — 각각 `-c key=value` 인자로 전달.
    /// 값은 codex가 TOML로 파싱; TOML 실패 시 raw string. 예:
    ///   ("model", "qwen3.6-35b-a3b") → 문자열로 파싱
    ///   ("model_provider", "local-vllm")
    pub config_overrides: Vec<(String, String)>,
    /// codex 자식 프로세스에 추가/덮어쓸 환경변수.
    /// codex provider 설정의 `env_key`를 만족시키는 데 사용 (vLLM처럼 키가 실제론
    /// 안 필요한 경우에도 codex가 빈 값 검증을 하므로 dummy라도 필요).
    pub env: Vec<(String, String)>,
}

impl SpawnOptions {
    /// 로컬 vLLM(또는 다른 OpenAI 호환 엔드포인트) 프로바이더 사용을 위한 헬퍼.
    /// `model_provider`가 config.toml에 미리 정의되어 있어야 한다.
    pub fn with_provider_model(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            config_overrides: vec![
                ("model_provider".into(), provider.into()),
                ("model".into(), model.into()),
            ],
            env: Vec::new(),
        }
    }

    pub fn push(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config_overrides.push((key.into(), value.into()));
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
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

/// macOS/Linux 한정 — Windows의 PATHEXT/.exe 처리는 안 함 (Stcode는 macOS 전용).
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
