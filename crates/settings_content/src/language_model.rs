use crate::merge_from::MergeFrom;
use collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

use std::sync::Arc;

#[with_fallible_options]
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, JsonSchema, MergeFrom)]
pub struct AllLanguageModelSettingsContent {
    pub deepseek: Option<DeepSeekSettingsContent>,
    pub openai_compatible: Option<HashMap<Arc<str>, OpenAiCompatibleSettingsContent>>,
    #[serde(rename = "zed.dev")]
    pub zed_dot_dev: Option<ZedDotDevSettingsContent>,
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
pub struct DeepSeekSettingsContent {
    pub api_url: Option<String>,
    pub available_models: Option<Vec<DeepSeekAvailableModel>>,
}

#[with_fallible_options]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct DeepSeekAvailableModel {
    pub name: String,
    pub display_name: Option<String>,
    pub max_tokens: u64,
    pub max_output_tokens: Option<u64>,
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
