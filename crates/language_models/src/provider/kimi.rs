use anyhow::Result;
use collections::BTreeMap;
use credentials_provider::CredentialsProvider;
use futures::{FutureExt, StreamExt, future::BoxFuture};
use gpui::{AnyView, App, AsyncApp, Context, Entity, Task, Window};
use http_client::HttpClient;
use language_model::{
    ApiKeyState, AuthenticateError, EnvVar, IconOrSvg, LanguageModel, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelId, LanguageModelName, LanguageModelProvider,
    LanguageModelProviderId, LanguageModelProviderName, LanguageModelProviderState,
    LanguageModelRequest, LanguageModelToolChoice, LanguageModelToolSchemaFormat, RateLimiter,
    env_var,
};
use open_ai::ResponseStreamEvent;
use serde_json::{Map, Value};
pub use settings::KimiAvailableModel as AvailableModel;
use settings::{Settings, SettingsStore};
use std::sync::{Arc, LazyLock};
use ui::{ButtonLink, ConfiguredApiCard, List, ListBulletItem, prelude::*};
use ui_input::InputField;
use util::ResultExt;

const PROVIDER_ID: LanguageModelProviderId = LanguageModelProviderId::new("kimi");
const PROVIDER_NAME: LanguageModelProviderName = LanguageModelProviderName::new("Kimi Code");

const KIMI_API_URL: &str = "https://api.kimi.com/coding/v1";
const DEFAULT_MODEL_ID: &str = "kimi-for-coding";
const DEFAULT_MODEL_DISPLAY_NAME: &str = "Kimi for Coding";
// Kimi K2.6 backing model — `kimi-for-coding` is a stable alias that auto-updates.
const DEFAULT_MAX_TOKENS: u64 = 262_144;
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 16_384;

const API_KEY_ENV_VAR_NAME: &str = "KIMI_API_KEY";
static API_KEY_ENV_VAR: LazyLock<EnvVar> = env_var!(API_KEY_ENV_VAR_NAME);

#[derive(Default, Clone, Debug, PartialEq)]
pub struct KimiSettings {
    pub api_url: String,
    pub available_models: Vec<AvailableModel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    name: String,
    display_name: Option<String>,
    max_tokens: u64,
    max_output_tokens: Option<u64>,
}

impl Model {
    fn default_for_coding() -> Self {
        Self {
            name: DEFAULT_MODEL_ID.to_string(),
            display_name: Some(DEFAULT_MODEL_DISPLAY_NAME.to_string()),
            max_tokens: DEFAULT_MAX_TOKENS,
            max_output_tokens: Some(DEFAULT_MAX_OUTPUT_TOKENS),
        }
    }

    fn id(&self) -> &str {
        &self.name
    }

    fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

pub struct KimiLanguageModelProvider {
    http_client: Arc<dyn HttpClient>,
    state: Entity<State>,
}

pub struct State {
    api_key_state: ApiKeyState,
    credentials_provider: Arc<dyn CredentialsProvider>,
}

impl State {
    fn is_authenticated(&self) -> bool {
        self.api_key_state.has_key()
    }

    fn set_api_key(&mut self, api_key: Option<String>, cx: &mut Context<Self>) -> Task<Result<()>> {
        let credentials_provider = self.credentials_provider.clone();
        let api_url = KimiLanguageModelProvider::api_url(cx);
        self.api_key_state.store(
            api_url,
            api_key,
            |this| &mut this.api_key_state,
            credentials_provider,
            cx,
        )
    }

    fn authenticate(&mut self, cx: &mut Context<Self>) -> Task<Result<(), AuthenticateError>> {
        let credentials_provider = self.credentials_provider.clone();
        let api_url = KimiLanguageModelProvider::api_url(cx);
        self.api_key_state.load_if_needed(
            api_url,
            |this| &mut this.api_key_state,
            credentials_provider,
            cx,
        )
    }
}

impl KimiLanguageModelProvider {
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        credentials_provider: Arc<dyn CredentialsProvider>,
        cx: &mut App,
    ) -> Self {
        let state = cx.new(|cx| {
            cx.observe_global::<SettingsStore>(|this: &mut State, cx| {
                let credentials_provider = this.credentials_provider.clone();
                let api_url = Self::api_url(cx);
                this.api_key_state.handle_url_change(
                    api_url,
                    |this| &mut this.api_key_state,
                    credentials_provider,
                    cx,
                );
                cx.notify();
            })
            .detach();
            State {
                api_key_state: ApiKeyState::new(Self::api_url(cx), (*API_KEY_ENV_VAR).clone()),
                credentials_provider,
            }
        });

        Self { http_client, state }
    }

    fn create_language_model(&self, model: Model) -> Arc<dyn LanguageModel> {
        Arc::new(KimiLanguageModel {
            id: LanguageModelId::from(model.id().to_string()),
            model,
            state: self.state.clone(),
            http_client: self.http_client.clone(),
            request_limiter: RateLimiter::new(4),
        })
    }

    fn settings(cx: &App) -> &KimiSettings {
        &crate::AllLanguageModelSettings::get_global(cx).kimi
    }

    fn api_url(cx: &App) -> SharedString {
        let api_url = &Self::settings(cx).api_url;
        if api_url.is_empty() {
            KIMI_API_URL.into()
        } else {
            SharedString::new(api_url.as_str())
        }
    }
}

impl LanguageModelProviderState for KimiLanguageModelProvider {
    type ObservableEntity = State;

    fn observable_entity(&self) -> Option<Entity<Self::ObservableEntity>> {
        Some(self.state.clone())
    }
}

impl LanguageModelProvider for KimiLanguageModelProvider {
    fn id(&self) -> LanguageModelProviderId {
        PROVIDER_ID
    }

    fn name(&self) -> LanguageModelProviderName {
        PROVIDER_NAME
    }

    fn icon(&self) -> IconOrSvg {
        IconOrSvg::Icon(IconName::AiOpenAiCompat)
    }

    fn default_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        Some(self.create_language_model(Model::default_for_coding()))
    }

    fn default_fast_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        Some(self.create_language_model(Model::default_for_coding()))
    }

    fn provided_models(&self, cx: &App) -> Vec<Arc<dyn LanguageModel>> {
        let mut models: BTreeMap<String, Model> = BTreeMap::default();

        let default = Model::default_for_coding();
        models.insert(default.id().to_string(), default);

        for model in &Self::settings(cx).available_models {
            models.insert(
                model.name.clone(),
                Model {
                    name: model.name.clone(),
                    display_name: model.display_name.clone(),
                    max_tokens: model.max_tokens,
                    max_output_tokens: model.max_output_tokens,
                },
            );
        }

        models
            .into_values()
            .map(|model| self.create_language_model(model))
            .collect()
    }

    fn is_authenticated(&self, cx: &App) -> bool {
        self.state.read(cx).is_authenticated()
    }

    fn authenticate(&self, cx: &mut App) -> Task<Result<(), AuthenticateError>> {
        self.state.update(cx, |state, cx| state.authenticate(cx))
    }

    fn configuration_view(
        &self,
        _target_agent: language_model::ConfigurationViewTargetAgent,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        cx.new(|cx| ConfigurationView::new(self.state.clone(), window, cx))
            .into()
    }

    fn reset_credentials(&self, cx: &mut App) -> Task<Result<()>> {
        self.state
            .update(cx, |state, cx| state.set_api_key(None, cx))
    }
}

pub struct KimiLanguageModel {
    id: LanguageModelId,
    model: Model,
    state: Entity<State>,
    http_client: Arc<dyn HttpClient>,
    request_limiter: RateLimiter,
}

impl KimiLanguageModel {
    fn stream_completion(
        &self,
        request: open_ai::Request,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            futures::stream::BoxStream<'static, Result<ResponseStreamEvent>>,
            LanguageModelCompletionError,
        >,
    > {
        let http_client = self.http_client.clone();

        let (api_key, api_url) = self.state.read_with(cx, |state, cx| {
            let api_url = KimiLanguageModelProvider::api_url(cx);
            (state.api_key_state.key(&api_url), api_url)
        });

        let future = self.request_limiter.stream(async move {
            let provider = PROVIDER_NAME;
            let Some(api_key) = api_key else {
                return Err(LanguageModelCompletionError::NoApiKey { provider });
            };
            let request = open_ai::stream_completion(
                http_client.as_ref(),
                provider.0.as_str(),
                &api_url,
                &api_key,
                request,
            );
            let response = request.await?;
            Ok(response)
        });

        async move { Ok(future.await?.boxed()) }.boxed()
    }
}

impl LanguageModel for KimiLanguageModel {
    fn id(&self) -> LanguageModelId {
        self.id.clone()
    }

    fn name(&self) -> LanguageModelName {
        LanguageModelName::from(self.model.display_name().to_string())
    }

    fn provider_id(&self) -> LanguageModelProviderId {
        PROVIDER_ID
    }

    fn provider_name(&self) -> LanguageModelProviderName {
        PROVIDER_NAME
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_images(&self) -> bool {
        false
    }

    fn supports_streaming_tools(&self) -> bool {
        true
    }

    fn supports_tool_choice(&self, choice: LanguageModelToolChoice) -> bool {
        match choice {
            LanguageModelToolChoice::Auto | LanguageModelToolChoice::None => true,
            // Moonshot's tool_choice currently does not accept "required".
            LanguageModelToolChoice::Any => false,
        }
    }

    fn tool_input_format(&self) -> LanguageModelToolSchemaFormat {
        LanguageModelToolSchemaFormat::JsonSchema
    }

    fn supports_thinking(&self) -> bool {
        false
    }

    fn supports_split_token_display(&self) -> bool {
        true
    }

    fn telemetry_id(&self) -> String {
        format!("kimi/{}", self.model.id())
    }

    fn max_token_count(&self) -> u64 {
        self.model.max_tokens
    }

    fn max_output_tokens(&self) -> Option<u64> {
        self.model.max_output_tokens
    }

    fn stream_completion(
        &self,
        request: LanguageModelRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            futures::stream::BoxStream<
                'static,
                Result<LanguageModelCompletionEvent, LanguageModelCompletionError>,
            >,
            LanguageModelCompletionError,
        >,
    > {
        let mut request = open_ai::completion::into_open_ai(
            request,
            self.model.id(),
            false,
            false,
            self.max_output_tokens(),
            None,
            false,
        );
        for tool in &mut request.tools {
            let open_ai::ToolDefinition::Function { function } = tool;
            if let Some(parameters) = &mut function.parameters {
                kimi_normalize_tool_schema(parameters);
            }
        }
        let completions = self.stream_completion(request, cx);
        async move {
            let mapper = open_ai::completion::OpenAiEventMapper::new();
            Ok(mapper.map_stream(completions.await?).boxed())
        }
        .boxed()
    }
}

/// Rewrites a JSON Schema in-place so it satisfies Moonshot's "Kimi-flavored"
/// JSON schema validator, which is a stricter subset of standard JSON Schema.
///
/// The known constraints applied here:
///   1. `enum` arrays must not contain `null` (rejects `Option<Enum>` schemas).
///   2. Every property in a `properties` map must declare a `type`.
///   3. `type` may not appear on the parent of an `anyOf` — it must be
///      pushed down into each branch.
///
/// See https://github.com/MoonshotAI/kimi-cli/issues/1595 for the broader
/// incompatibility surface.
fn kimi_normalize_tool_schema(value: &mut Value) {
    strip_null_from_enums(value);
    push_type_into_any_of_branches(value);
    ensure_type_on_properties(value);
}

fn strip_null_from_enums(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(enum_value) = map.get_mut("enum")
                && let Value::Array(items) = enum_value
            {
                items.retain(|item| !item.is_null());
            }
            for (_, child) in map.iter_mut() {
                strip_null_from_enums(child);
            }
        }
        Value::Array(arr) => {
            for child in arr.iter_mut() {
                strip_null_from_enums(child);
            }
        }
        _ => {}
    }
}

fn push_type_into_any_of_branches(value: &mut Value) {
    if let Value::Object(map) = value {
        if let (Some(Value::Array(_)), Some(_)) = (map.get("anyOf"), map.get("type")) {
            let parent_type = map.remove("type");
            if let (Some(Value::Array(branches)), Some(parent_type)) =
                (map.get_mut("anyOf"), parent_type)
            {
                for branch in branches.iter_mut() {
                    if let Value::Object(branch_map) = branch
                        && !branch_map.contains_key("type")
                    {
                        branch_map.insert("type".to_string(), parent_type.clone());
                    }
                }
            }
        }
        for (_, child) in map.iter_mut() {
            push_type_into_any_of_branches(child);
        }
    } else if let Value::Array(arr) = value {
        for child in arr.iter_mut() {
            push_type_into_any_of_branches(child);
        }
    }
}

fn ensure_type_on_properties(value: &mut Value) {
    if let Value::Object(map) = value {
        if let Some(Value::Object(properties)) = map.get_mut("properties") {
            for (_, prop) in properties.iter_mut() {
                if let Value::Object(prop_map) = prop {
                    inject_default_type(prop_map);
                }
            }
        }
        for (_, child) in map.iter_mut() {
            ensure_type_on_properties(child);
        }
    } else if let Value::Array(arr) = value {
        for child in arr.iter_mut() {
            ensure_type_on_properties(child);
        }
    }
}

fn inject_default_type(map: &mut Map<String, Value>) {
    if map.contains_key("type")
        || map.contains_key("anyOf")
        || map.contains_key("allOf")
        || map.contains_key("oneOf")
        || map.contains_key("$ref")
    {
        return;
    }
    map.insert("type".to_string(), Value::String("string".to_string()));
}

struct ConfigurationView {
    api_key_editor: Entity<InputField>,
    state: Entity<State>,
    load_credentials_task: Option<Task<()>>,
}

impl ConfigurationView {
    fn new(state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let api_key_editor =
            cx.new(|cx| InputField::new(window, cx, "sk-...").label("API key"));

        cx.observe(&state, |_, _, cx| {
            cx.notify();
        })
        .detach();

        let load_credentials_task = Some(cx.spawn_in(window, {
            let state = state.clone();
            async move |this, cx| {
                if let Some(task) = Some(state.update(cx, |state, cx| state.authenticate(cx))) {
                    let _ = task.await;
                }
                this.update(cx, |this, cx| {
                    this.load_credentials_task = None;
                    cx.notify();
                })
                .log_err();
            }
        }));

        Self {
            api_key_editor,
            state,
            load_credentials_task,
        }
    }

    fn save_api_key(&mut self, _: &menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let api_key = self.api_key_editor.read(cx).text(cx).trim().to_string();
        if api_key.is_empty() {
            return;
        }

        self.api_key_editor
            .update(cx, |editor, cx| editor.set_text("", window, cx));

        let state = self.state.clone();
        cx.spawn_in(window, async move |_, cx| {
            state
                .update(cx, |state, cx| state.set_api_key(Some(api_key), cx))
                .await
        })
        .detach_and_log_err(cx);
    }

    fn reset_api_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.api_key_editor
            .update(cx, |input, cx| input.set_text("", window, cx));

        let state = self.state.clone();
        cx.spawn_in(window, async move |_, cx| {
            state
                .update(cx, |state, cx| state.set_api_key(None, cx))
                .await
        })
        .detach_and_log_err(cx);
    }

    fn should_render_editor(&self, cx: &mut Context<Self>) -> bool {
        !self.state.read(cx).is_authenticated()
    }
}

impl Render for ConfigurationView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let env_var_set = self.state.read(cx).api_key_state.is_from_env_var();
        let configured_card_label = if env_var_set {
            format!("API key set in {API_KEY_ENV_VAR_NAME} environment variable")
        } else {
            let api_url = KimiLanguageModelProvider::api_url(cx);
            if api_url == KIMI_API_URL {
                "API key configured".to_string()
            } else {
                format!("API key configured for {}", api_url)
            }
        };

        let api_key_section = if self.should_render_editor(cx) {
            v_flex()
                .on_action(cx.listener(Self::save_api_key))
                .child(Label::new(
                    "To use the agent with Kimi Code, paste an API key from your Kimi Code Console — not from platform.kimi.ai (which is a separate pay-per-token product).",
                ))
                .child(
                    List::new()
                        .child(
                            ListBulletItem::new("")
                                .child(Label::new("Create one by visiting"))
                                .child(ButtonLink::new(
                                    "Kimi Code Console",
                                    "https://www.kimi.com/code",
                                )),
                        )
                        .child(ListBulletItem::new(
                            "Paste your API key below and hit enter to start using the agent",
                        )),
                )
                .child(self.api_key_editor.clone())
                .child(
                    Label::new(format!(
                        "You can also set the {API_KEY_ENV_VAR_NAME} environment variable and restart Stcode."
                    ))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
                )
                .into_any_element()
        } else {
            ConfiguredApiCard::new(configured_card_label)
                .disabled(env_var_set)
                .when(env_var_set, |this| {
                    this.tooltip_label(format!(
                        "To reset your API key, unset the {API_KEY_ENV_VAR_NAME} environment variable."
                    ))
                })
                .on_click(cx.listener(|this, _, window, cx| this.reset_api_key(window, cx)))
                .into_any_element()
        };

        if self.load_credentials_task.is_some() {
            div().child(Label::new("Loading credentials…")).into_any()
        } else {
            v_flex().size_full().child(api_key_section).into_any()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_null_from_enum_array() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "role": {
                    "type": "string",
                    "enum": ["explore", "plan", null]
                }
            }
        });

        kimi_normalize_tool_schema(&mut schema);

        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": {
                    "role": {
                        "type": "string",
                        "enum": ["explore", "plan"]
                    }
                }
            })
        );
    }

    #[test]
    fn strips_null_from_nested_enums() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "outer": {
                    "type": "object",
                    "properties": {
                        "inner": {
                            "type": "string",
                            "enum": ["a", null, "b"]
                        }
                    }
                }
            }
        });

        kimi_normalize_tool_schema(&mut schema);

        assert_eq!(
            schema["properties"]["outer"]["properties"]["inner"]["enum"],
            json!(["a", "b"])
        );
    }

    #[test]
    fn pushes_type_into_any_of_branches() {
        let mut schema = json!({
            "anyOf": [
                {"const": "a"},
                {"const": "b"}
            ],
            "type": "string"
        });

        kimi_normalize_tool_schema(&mut schema);

        assert_eq!(
            schema,
            json!({
                "anyOf": [
                    {"const": "a", "type": "string"},
                    {"const": "b", "type": "string"}
                ]
            })
        );
    }

    #[test]
    fn does_not_overwrite_branch_type() {
        let mut schema = json!({
            "anyOf": [
                {"const": "a", "type": "string"},
                {"const": 1, "type": "integer"}
            ],
            "type": "string"
        });

        kimi_normalize_tool_schema(&mut schema);

        assert_eq!(
            schema["anyOf"],
            json!([
                {"const": "a", "type": "string"},
                {"const": 1, "type": "integer"}
            ])
        );
        assert!(!schema.as_object().unwrap().contains_key("type"));
    }

    #[test]
    fn injects_default_type_on_properties() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "missing": {"description": "no type"},
                "present": {"type": "integer"},
                "with_any_of": {"anyOf": [{"type": "string"}]}
            }
        });

        kimi_normalize_tool_schema(&mut schema);

        assert_eq!(schema["properties"]["missing"]["type"], json!("string"));
        assert_eq!(schema["properties"]["present"]["type"], json!("integer"));
        assert!(
            !schema["properties"]["with_any_of"]
                .as_object()
                .unwrap()
                .contains_key("type")
        );
    }

    #[test]
    fn handles_real_world_spawn_agent_role_field() {
        // Mirrors the SpawnAgentToolInput schema where `role: Option<SubagentRole>`
        // produces a `null` entry in the enum that Moonshot rejects.
        let mut schema = json!({
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "role": {
                    "type": "string",
                    "enum": ["explore", "plan", "task", "review", "verify", null]
                },
                "message": {"type": "string"}
            }
        });

        kimi_normalize_tool_schema(&mut schema);

        assert_eq!(
            schema["properties"]["role"]["enum"],
            json!(["explore", "plan", "task", "review", "verify"])
        );
    }
}
