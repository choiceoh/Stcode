use crate::merge_from::MergeFrom;
use collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

use std::sync::Arc;

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AllLanguageModelSettingsContent {
    pub anthropic: Option<AnthropicSettingsContent>,
    pub bedrock: Option<AmazonBedrockSettingsContent>,
    pub google: Option<GoogleSettingsContent>,
    pub opencode: Option<OpenCodeSettingsContent>,
    pub openai: Option<OpenAiSettingsContent>,
    pub openai_compatible: Option<HashMap<Arc<str>, OpenAiCompatibleSettingsContent>>,
    pub vercel_ai_gateway: Option<VercelAiGatewaySettingsContent>,
    pub x_ai: Option<XAiSettingsContent>,
    #[serde(rename = "zed.dev")]
    pub zed_dot_dev: Option<ZedDotDevSettingsContent>,
}

#[with_fallible_options]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AnthropicSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<AnthropicAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct AnthropicAvailableModel {
    /// The model's name in the Anthropic API. e.g. claude-3-5-sonnet-latest, claude-3-opus-20240229, etc
    pub name: String,
    /// The model's name in Zed's UI, such as in the model selector dropdown menu in the agent panel.
    pub display_name: Option<String>,
    /// The model's context window size.
    pub max_tokens: u64,
    /// A model `name` to substitute when calling tools, in case the primary model doesn't support tool calling.
    pub tool_override: Option<String>,
    /// Configuration of Anthropic's caching API.
    pub cache_configuration: Option<LanguageModelCacheConfiguration>,
    pub max_output_tokens: Option<u64>,
    #[serde(serialize_with = "crate::serialize_optional_f32_with_two_decimal_places")]
    pub default_temperature: Option<f32>,
    #[serde(default)]
    pub extra_beta_headers: Vec<String>,
    /// The model's mode (e.g. thinking)
    pub mode: Option<ModelMode>,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AmazonBedrockSettingsContent {
    pub available_models: Option<Vec<BedrockAvailableModel>>,
    pub endpoint_url: Option<String>,
    pub region: Option<String>,
    pub profile: Option<String>,
    pub authentication_method: Option<BedrockAuthMethodContent>,
    pub allow_global: Option<bool>,
    /// Enable the 1M token extended context window beta for supported Anthropic models.
    pub allow_extended_context: Option<bool>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct BedrockAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub cache_configuration: Option<LanguageModelCacheConfiguration>,
    pub max_output_tokens: Option<u64>,
    #[serde(serialize_with = "crate::serialize_optional_f32_with_two_decimal_places")]
    pub default_temperature: Option<f32>,
    pub mode: Option<ModelMode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub enum BedrockAuthMethodContent {
    #[serde(rename = "named_profile")]
    NamedProfile,
    #[serde(rename = "sso")]
    SingleSignOn,
    #[serde(rename = "api_key")]
    ApiKey,
    /// IMDSv2, PodIdentity, env vars, etc.
    #[serde(rename = "default")]
    Automatic,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct OpenCodeSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<OpenCodeAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenCodeAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
    /// The API protocol to use for this model: "anthropic", "openai_responses", "openai_chat", or "google".
    pub protocol: String,
}

#[with_fallible_options]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct OpenAiSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<OpenAiAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub reasoning_effort: Option<OpenAiReasoningEffort>,
    #[serde(default)]
    pub capabilities: OpenAiModelCapabilities,
}

pub use language_model_core::ReasoningEffort as OpenAiReasoningEffort;

impl MergeFrom for OpenAiReasoningEffort {
    fn merge_from(&mut self, other: &Self) {
        *self = *other;
    }
}

#[with_fallible_options]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct OpenAiCompatibleSettingsContent {
    pub api_url: String,
    pub available_models: Vec<OpenAiCompatibleAvailableModel>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiModelCapabilities {
    #[serde(default = "default_true")]
    pub chat_completions: bool,
}

impl Default for OpenAiModelCapabilities {
    fn default() -> Self {
        Self {
            chat_completions: default_true(),
        }
    }
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiCompatibleAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub reasoning_effort: Option<OpenAiReasoningEffort>,
    #[serde(default)]
    pub capabilities: OpenAiCompatibleModelCapabilities,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct OpenAiCompatibleModelCapabilities {
    pub tools: bool,
    pub images: bool,
    pub parallel_tool_calls: bool,
    pub prompt_cache_key: bool,
    #[serde(default = "default_true")]
    pub chat_completions: bool,
    #[serde(default)]
    pub interleaved_reasoning: bool,
}

impl Default for OpenAiCompatibleModelCapabilities {
    fn default() -> Self {
        Self {
            tools: true,
            images: false,
            parallel_tool_calls: false,
            prompt_cache_key: false,
            chat_completions: default_true(),
            interleaved_reasoning: false,
        }
    }
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct VercelAiGatewaySettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<VercelAiGatewayAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct VercelAiGatewayAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    #[serde(default)]
    pub capabilities: OpenAiCompatibleModelCapabilities,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct GoogleSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<GoogleAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct GoogleAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub mode: Option<ModelMode>,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct XAiSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<XaiAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct XaiAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub supports_images: Option<bool>,
    pub supports_tools: Option<bool>,
    pub parallel_tool_calls: Option<bool>,
}

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct ZedDotDevSettingsContent {
    pub available_models: Option<Vec<ZedDotDevAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct ZedDotDevAvailableModel {
    /// The provider of the language model.
    pub provider: ZedDotDevAvailableProvider,
    /// The model's name in the provider's API. e.g. claude-3-5-sonnet-20240620
    pub name: String,
    /// The name displayed in the UI, such as in the agent panel model dropdown menu.
    pub display_name: Option<String>,
    /// The size of the context window, indicating the maximum number of tokens the model can process.
    pub max_tokens: usize,
    /// The maximum number of output tokens allowed by the model.
    pub max_output_tokens: Option<u64>,
    /// The maximum number of completion tokens allowed by the model (o1-* only)
    pub max_completion_tokens: Option<u64>,
    /// Override this model with a different Anthropic model for tool calls.
    pub tool_override: Option<String>,
    /// Indicates whether this custom model supports caching.
    pub cache_configuration: Option<LanguageModelCacheConfiguration>,
    /// The default temperature to use for this model.
    #[serde(serialize_with = "crate::serialize_optional_f32_with_two_decimal_places")]
    pub default_temperature: Option<f32>,
    /// Any extra beta headers to provide when using the model.
    #[serde(default)]
    pub extra_beta_headers: Vec<String>,
    /// The model's mode (e.g. thinking)
    pub mode: Option<ModelMode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
#[serde(rename_all = "lowercase")]
pub enum ZedDotDevAvailableProvider {
    Anthropic,
    OpenAi,
    Google,
}

fn default_true() -> bool {
    true
}

/// Configuration for caching language model messages.
#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct LanguageModelCacheConfiguration {
    pub max_cache_anchors: usize,
    pub should_speculate: bool,
    pub min_total_token: u64,
}

pub use language_model_core::ModelMode;

impl MergeFrom for ModelMode {
    fn merge_from(&mut self, other: &Self) {
        *self = *other;
    }
}
