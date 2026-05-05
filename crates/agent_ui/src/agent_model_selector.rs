use crate::{
    ModelUsageContext,
    language_model_selector::{LanguageModelSelector, language_model_selector},
    ui::ModelSelectorTooltip,
};
use fs::Fs;
use gpui::{Entity, FocusHandle, SharedString, Subscription};
use language_model::{IconOrSvg, LanguageModelRegistry};
use picker::popover_menu::PickerPopoverMenu;
use settings::SettingsStore;
use std::sync::Arc;
use ui::{PopoverMenuHandle, Tooltip, prelude::*};

pub struct AgentModelSelector {
    selector: Entity<LanguageModelSelector>,
    menu_handle: PopoverMenuHandle<LanguageModelSelector>,
    empty_model_label: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl AgentModelSelector {
    pub(crate) fn new(
        fs: Arc<dyn Fs>,
        menu_handle: PopoverMenuHandle<LanguageModelSelector>,
        focus_handle: FocusHandle,
        model_usage_context: ModelUsageContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let empty_model_label = model_usage_context.empty_model_label();
        let subscriptions = vec![
            cx.observe_global::<SettingsStore>(|_, cx| cx.notify()),
            cx.subscribe(
                &LanguageModelRegistry::global(cx),
                |_, _, event: &language_model::Event, cx| {
                    if matches!(
                        event,
                        language_model::Event::DefaultModelChanged
                            | language_model::Event::InlineAssistantModelChanged
                            | language_model::Event::ProviderStateChanged(_)
                            | language_model::Event::AddedProvider(_)
                            | language_model::Event::RemovedProvider(_)
                            | language_model::Event::ProvidersChanged
                    ) {
                        cx.notify();
                    }
                },
            ),
        ];
        Self {
            selector: cx.new(move |cx| {
                language_model_selector(
                    {
                        let model_context = model_usage_context.clone();
                        move |cx| model_context.configured_model(cx)
                    },
                    {
                        let fs = fs.clone();
                        let model_context = model_usage_context.clone();
                        move |model, cx| {
                            model_context.set_model(fs.clone(), model, cx);
                        }
                    },
                    {
                        let fs = fs.clone();
                        move |model, should_be_favorite, cx| {
                            crate::favorite_models::toggle_in_settings(
                                model,
                                should_be_favorite,
                                fs.clone(),
                                cx,
                            );
                        }
                    },
                    true, // Use popover styles for picker
                    focus_handle.clone(),
                    window,
                    cx,
                )
            }),
            menu_handle,
            empty_model_label,
            _subscriptions: subscriptions,
        }
    }

    pub fn toggle(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_handle.toggle(window, cx);
    }

    pub fn active_model(&self, cx: &App) -> Option<language_model::ConfiguredModel> {
        self.selector.read(cx).delegate.active_model(cx)
    }

    pub fn cycle_favorite_models(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.selector.update(cx, |selector, cx| {
            selector.delegate.cycle_favorite_models(window, cx);
        });
    }
}

impl Render for AgentModelSelector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.selector.read(cx).delegate.active_model(cx);
        let model_name = model
            .as_ref()
            .map(|model| model.model.name().0)
            .unwrap_or_else(|| self.empty_model_label.clone());

        let provider_icon = model.as_ref().map(|model| model.provider.icon());
        let color = if self.menu_handle.is_deployed() {
            Color::Accent
        } else {
            Color::Muted
        };

        let show_cycle_row = self.selector.read(cx).delegate.favorites_count() > 1;

        let tooltip = Tooltip::element({
            move |_, _cx| {
                ModelSelectorTooltip::new()
                    .show_cycle_row(show_cycle_row)
                    .into_any_element()
            }
        });

        PickerPopoverMenu::new(
            self.selector.clone(),
            Button::new("active-model", model_name)
                .label_size(LabelSize::Small)
                .color(color)
                .when_some(provider_icon, |this, icon| {
                    this.start_icon(
                        match icon {
                            IconOrSvg::Svg(path) => Icon::from_external_svg(path),
                            IconOrSvg::Icon(name) => Icon::new(name),
                        }
                        .color(color)
                        .size(IconSize::XSmall),
                    )
                })
                .end_icon(
                    Icon::new(IconName::ChevronDown)
                        .color(color)
                        .size(IconSize::XSmall),
                ),
            tooltip,
            gpui::Anchor::TopRight,
            cx,
        )
        .with_handle(self.menu_handle.clone())
        .offset(gpui::Point {
            x: px(0.0),
            y: px(2.0),
        })
        .render(window, cx)
    }
}
