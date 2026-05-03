//! 사용자 설정 — model/provider 등. macOS Application Support 폴더에 toml로.
//!
//! 위치: `~/Library/Application Support/Stcode/settings.toml`
//! 첫 실행이거나 파일이 없으면 [`Settings::default`] (vLLM + qwen).
//!
//! provider/model 호환 필드는 유지하되, 실제 라우팅은 main/sub agent 모델 기본값으로 한다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// codex `config.toml` 에 정의된 provider 이름 (예: "local-vllm", "openai").
    pub provider: String,
    /// 예전 설정 호환용 모델 식별자. 새 설정에선 main_model과 함께 갱신한다.
    pub model: String,
    /// 사용자를 상대하고 전체 작업을 조율하는 메인 에이전트 모델.
    #[serde(default)]
    pub main_model: String,
    /// 실제 반복/실행 작업을 맡는 서브 에이전트 기본 모델.
    #[serde(default)]
    pub sub_model: String,
    /// 최근 열었던 프로젝트. GUI에서 바로 다시 열 수 있게 경로만 저장한다.
    #[serde(default)]
    pub recent_projects: Vec<String>,
}

const MAX_RECENT_PROJECTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentModelRole {
    Main,
    Sub,
}

impl Default for Settings {
    fn default() -> Self {
        let model = "qwen3.6-35b-a3b".to_string();
        Self {
            provider: "local-vllm".into(),
            model: model.clone(),
            main_model: model.clone(),
            sub_model: model,
            recent_projects: Vec::new(),
        }
    }
}

impl Settings {
    /// 오래된 settings.toml에는 main_model/sub_model이 없을 수 있다. 그런 경우 기존 model로
    /// 채워서 사용자가 설정 파일을 지우지 않아도 자연스럽게 업그레이드한다.
    pub fn normalized(mut self) -> Self {
        if self.model.trim().is_empty() {
            self.model = Settings::default().model;
        }
        if self.main_model.trim().is_empty() {
            self.main_model = self.model.clone();
        }
        if self.sub_model.trim().is_empty() {
            self.sub_model = self.main_model.clone();
        }
        let mut recent_projects = Vec::new();
        for project in self.recent_projects {
            if project.trim().is_empty() || recent_projects.contains(&project) {
                continue;
            }
            recent_projects.push(project);
            if recent_projects.len() == MAX_RECENT_PROJECTS {
                break;
            }
        }
        self.recent_projects = recent_projects;
        self
    }

    pub fn model_for_role(&self, role: AgentModelRole) -> &str {
        match role {
            AgentModelRole::Main => &self.main_model,
            AgentModelRole::Sub => &self.sub_model,
        }
    }

    pub fn remember_recent_project(&mut self, path: &Path) {
        let path = path.to_string_lossy().into_owned();
        if path.trim().is_empty() {
            return;
        }
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        if self.recent_projects.len() > MAX_RECENT_PROJECTS {
            self.recent_projects.truncate(MAX_RECENT_PROJECTS);
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
        Ok(s) => match toml::from_str::<Settings>(&s) {
            Ok(settings) => settings.normalized(),
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
    let toml_str = toml::to_string_pretty(&settings.clone().normalized())?;
    std::fs::write(&path, toml_str)?;
    Ok(())
}

/// `~/Library/Application Support/Stcode/settings.toml` (macOS).
/// 다른 OS에선 `dirs::config_dir()` fallback.
pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("Stcode").join("settings.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_without_role_models_fall_back_to_model() {
        let parsed: Settings = toml::from_str(
            r#"
provider = "local-vllm"
model = "old-model"
"#,
        )
        .expect("parse settings");

        let normalized = parsed.normalized();

        assert_eq!(normalized.main_model, "old-model");
        assert_eq!(normalized.sub_model, "old-model");
        assert_eq!(normalized.model_for_role(AgentModelRole::Main), "old-model");
        assert_eq!(normalized.model_for_role(AgentModelRole::Sub), "old-model");
        assert!(normalized.recent_projects.is_empty());
    }

    #[test]
    fn role_models_are_preserved_when_present() {
        let parsed: Settings = toml::from_str(
            r#"
provider = "local-vllm"
model = "legacy"
main_model = "planner"
sub_model = "worker"
"#,
        )
        .expect("parse settings");

        let normalized = parsed.normalized();

        assert_eq!(normalized.main_model, "planner");
        assert_eq!(normalized.sub_model, "worker");
        assert_eq!(normalized.model_for_role(AgentModelRole::Main), "planner");
        assert_eq!(normalized.model_for_role(AgentModelRole::Sub), "worker");
    }

    #[test]
    fn recent_projects_are_deduped_and_capped() {
        let mut settings = Settings::default();
        for idx in 0..10 {
            settings.remember_recent_project(Path::new(&format!("/tmp/project-{idx}")));
        }
        settings.remember_recent_project(Path::new("/tmp/project-5"));

        assert_eq!(settings.recent_projects.first().unwrap(), "/tmp/project-5");
        assert_eq!(settings.recent_projects.len(), 8);
        assert_eq!(
            settings
                .recent_projects
                .iter()
                .filter(|path| path.as_str() == "/tmp/project-5")
                .count(),
            1
        );
    }
}
