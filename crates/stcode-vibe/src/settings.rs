//! 사용자 설정 — model/provider 등. macOS Application Support 폴더에 toml로.
//!
//! 위치: `~/Library/Application Support/Stcode/settings.toml`
//! 첫 실행이거나 파일이 없으면 [`Settings::default`] (vLLM + qwen).
//!
//! v1엔 model/provider 만. 미래 확장(테마, 시스템 prompt 등)은 같은 파일에 추가.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// codex `config.toml` 에 정의된 provider 이름 (예: "local-vllm", "openai").
    pub provider: String,
    /// 모델 식별자 (예: "qwen3.6-35b-a3b", "gpt-5.5").
    pub model: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "local-vllm".into(),
            model: "qwen3.6-35b-a3b".into(),
        }
    }
}

/// 디스크에서 로드. 없거나 깨지면 default + warn.
pub fn load() -> Settings {
    let path = match settings_path() {
        Some(p) => p,
        None => return Settings::default(),
    };
    if !path.exists() {
        return Settings::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => match toml::from_str(&s) {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!("settings.toml 파싱 실패 ({e}) — 기본값 사용");
                Settings::default()
            }
        },
        Err(e) => {
            tracing::warn!("settings.toml 읽기 실패 ({e}) — 기본값 사용");
            Settings::default()
        }
    }
}

/// 디스크에 저장. 디렉터리는 자동 생성.
pub fn save(settings: &Settings) -> anyhow::Result<()> {
    let path = settings_path()
        .ok_or_else(|| anyhow::anyhow!("Application Support 디렉터리를 찾을 수 없어요"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(settings)?;
    std::fs::write(&path, toml_str)?;
    Ok(())
}

/// `~/Library/Application Support/Stcode/settings.toml` (macOS).
/// 다른 OS에선 `dirs::config_dir()` fallback.
pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("Stcode").join("settings.toml"))
}
