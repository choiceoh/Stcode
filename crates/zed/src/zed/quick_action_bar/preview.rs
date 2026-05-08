use gpui::{AnyElement, Modifiers, WeakEntity};
use markdown_preview::{
    OpenPreview as MarkdownOpenPreview, OpenPreviewToTheSide as MarkdownOpenPreviewToTheSide,
    markdown_preview_view::MarkdownPreviewView,
};
use ui::{Tooltip, prelude::*, text_for_keystroke};
use workspace::Workspace;

use super::QuickActionBar;

impl QuickActionBar {
    pub fn render_preview_button(
        &self,
        workspace_handle: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let mut is_markdown = false;

        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                if MarkdownPreviewView::resolve_active_item_as_markdown_editor(workspace, cx)
                    .is_some()
                {
                    is_markdown = true;
                }
            });
        }

        if !is_markdown {
            return None;
        }

        let alt_click = gpui::Keystroke {
            key: "click".into(),
            modifiers: Modifiers::alt(),
            ..Default::default()
        };

        let button = IconButton::new("toggle-markdown-preview", IconName::Eye)
            .icon_size(IconSize::Small)
            .style(ButtonStyle::Subtle)
            .tooltip(move |_window, cx| {
                Tooltip::with_meta(
                    "Preview Markdown",
                    Some(&markdown_preview::OpenPreview as &dyn gpui::Action),
                    format!(
                        "{} to open in a split",
                        text_for_keystroke(&alt_click.modifiers, &alt_click.key, cx)
                    ),
                    cx,
                )
            })
            .on_click(move |_, window, cx| {
                if let Some(workspace) = workspace_handle.upgrade() {
                    workspace.update(cx, |_, cx| {
                        if window.modifiers().alt {
                            window.dispatch_action(
                                Box::new(MarkdownOpenPreviewToTheSide) as Box<dyn gpui::Action>,
                                cx,
                            );
                        } else {
                            window.dispatch_action(
                                Box::new(MarkdownOpenPreview) as Box<dyn gpui::Action>,
                                cx,
                            );
                        }
                    });
                }
            });

        Some(button.into_any_element())
    }
}
