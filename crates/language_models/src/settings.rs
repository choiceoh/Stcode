use std::sync::Arc;

use collections::HashMap;
use settings::RegisterSetting;

use crate::provider::{
    anthropic::AnthropicSettings, bedrock::AmazonBedrockSettings, cloud::ZedDotDevSettings,
    google::GoogleSettings, open_ai::OpenAiSettings,
    open_ai_compatible::OpenAiCompatibleSettings, opencode::OpenCodeSettings,
    vercel_ai_gateway::VercelAiGatewaySettings, x_ai::XAiSettings,
};

#[derive(Debug, RegisterSetting)]
pub struct AllLanguageModelSettings {
    pub anthropic: AnthropicSettings,
    pub bedrock: AmazonBedrockSettings,
    pub google: GoogleSettings,
    pub opencode: OpenCodeSettings,
    pub openai: OpenAiSettings,
    pub openai_compatible: HashMap<Arc<str>, OpenAiCompatibleSettings>,
    pub vercel_ai_gateway: VercelAiGatewaySettings,
    pub x_ai: XAiSettings,
    pub zed_dot_dev: ZedDotDevSettings,
}

impl settings::Settings for AllLanguageModelSettings {
    const PRESERVED_KEYS: Option<&'static [&'static str]> = Some(&["version"]);

    fn from_settings(content: &settings::SettingsContent) -> Self {
        let language_models = content.language_models.clone().unwrap();
        let anthropic = language_models.anthropic.unwrap();
        let bedrock = language_models.bedrock.unwrap();
        let google = language_models.google.unwrap();
        let opencode = language_models.opencode.unwrap();
        let openai = language_models.openai.unwrap();
        let openai_compatible = language_models.openai_compatible.unwrap();
        let vercel_ai_gateway = language_models.vercel_ai_gateway.unwrap();
        let x_ai = language_models.x_ai.unwrap();
        let zed_dot_dev = language_models.zed_dot_dev.unwrap();
        Self {
            anthropic: AnthropicSettings {
                api_url: anthropic.api_url.unwrap(),
                available_models: anthropic.available_models.unwrap_or_default(),
            },
            bedrock: AmazonBedrockSettings {
                available_models: bedrock.available_models.unwrap_or_default(),
                region: bedrock.region,
                endpoint: bedrock.endpoint_url,
                profile_name: bedrock.profile,
                role_arn: None,
                authentication_method: bedrock.authentication_method.map(Into::into),
                allow_global: bedrock.allow_global,
                allow_extended_context: bedrock.allow_extended_context,
            },
            google: GoogleSettings {
                api_url: google.api_url.unwrap(),
                available_models: google.available_models.unwrap_or_default(),
            },
            opencode: OpenCodeSettings {
                api_url: opencode.api_url.unwrap(),
                available_models: opencode.available_models.unwrap_or_default(),
            },
            openai: OpenAiSettings {
                api_url: openai.api_url.unwrap(),
                available_models: openai.available_models.unwrap_or_default(),
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
            vercel_ai_gateway: VercelAiGatewaySettings {
                api_url: vercel_ai_gateway.api_url.unwrap(),
                available_models: vercel_ai_gateway.available_models.unwrap_or_default(),
            },
            x_ai: XAiSettings {
                api_url: x_ai.api_url.unwrap(),
                available_models: x_ai.available_models.unwrap_or_default(),
            },
            zed_dot_dev: ZedDotDevSettings {
                available_models: zed_dot_dev.available_models.unwrap_or_default(),
            },
        }
    }
}
