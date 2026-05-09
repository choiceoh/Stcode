use std::sync::Arc;

use collections::HashMap;
use settings::RegisterSetting;

use crate::provider::{
    cloud::ZedDotDevSettings, deepseek::DeepSeekSettings,
    open_ai_compatible::OpenAiCompatibleSettings,
};

#[derive(Debug, RegisterSetting)]
pub struct AllLanguageModelSettings {
    pub deepseek: DeepSeekSettings,
    pub openai_compatible: HashMap<Arc<str>, OpenAiCompatibleSettings>,
    pub zed_dot_dev: ZedDotDevSettings,
}

impl settings::Settings for AllLanguageModelSettings {
    const PRESERVED_KEYS: Option<&'static [&'static str]> = Some(&["version"]);

    fn from_settings(content: &settings::SettingsContent) -> Self {
        let language_models = content.language_models.clone().unwrap();
        let deepseek = language_models.deepseek.unwrap();
        let openai_compatible = language_models.openai_compatible.unwrap();
        let zed_dot_dev = language_models.zed_dot_dev.unwrap();
        Self {
            deepseek: DeepSeekSettings {
                api_url: deepseek.api_url.unwrap_or_default(),
                available_models: deepseek.available_models.unwrap_or_default(),
            },
            openai_compatible: openai_compatible
                .into_iter()
                .map(|(key, value)| {
                    (
                        key,
                        OpenAiCompatibleSettings {
                            api_url: value.api_url,
                            available_models: value.available_models,
                        },
                    )
                })
                .collect(),
            zed_dot_dev: ZedDotDevSettings {
                available_models: zed_dot_dev.available_models.unwrap_or_default(),
            },
        }
    }
}
