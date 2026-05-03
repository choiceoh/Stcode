//! Read-only selectable text — multi-line wrap + character-level selection.
//! shape_text(wrap_width)로 multi-line, height auto-grow.

use std::ops::Range;

use gpui::{
    actions, div, fill, font, point, prelude::*, px, relative, rgba, App, Bounds, ClipboardItem,
    Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    ParentElement, Pixels, Point, Render, SharedString, Style, Styled, TextRun, UTF16Selection,
    UnderlineStyle, Window, WrappedLine,
};

actions!(selectable_text, [Copy, SelectAll]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineKind {
    Code,
    Bold,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineSpan {
    pub range: Range<usize>,
    pub kind: InlineKind,
}

/// 메인에서 한 번 호출 — Cmd+C / Cmd+A 키바인딩 등록.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, Some("SelectableText")),
        KeyBinding::new("cmd-a", SelectAll, Some("SelectableText")),
    ]);
}

pub struct SelectableText {
    content: SharedString,
    focus_handle: FocusHandle,
    selected_range: Range<usize>,
    selection_reversed: bool,
    last_layout: Option<WrappedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    last_height: Option<Pixels>,
    is_selecting: bool,
    text_color: u32,
    inline_spans: Vec<InlineSpan>,
}

impl SelectableText {
    pub fn new(content: impl Into<SharedString>, color: u32, cx: &mut Context<Self>) -> Self {
        Self {
            content: content.into(),
            focus_handle: cx.focus_handle(),
            selected_range: 0..0,
            selection_reversed: false,
            last_layout: None,
            last_bounds: None,
            last_height: None,
            is_selecting: false,
            text_color: color,
            inline_spans: Vec::new(),
        }
    }

    pub fn new_inline(
        content: impl Into<SharedString>,
        color: u32,
        inline_spans: Vec<InlineSpan>,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = content.into();
        let len = content.len();
        let mut inline_spans = inline_spans
            .into_iter()
            .filter(|span| span.range.start < span.range.end && span.range.end <= len)
            .collect::<Vec<_>>();
        inline_spans.sort_by_key(|span| (span.range.start, span.range.end));
        let mut last_end = 0;
        inline_spans.retain(|span| {
            if span.range.start < last_end {
                return false;
            }
            last_end = span.range.end;
            true
        });
        Self {
            content,
            focus_handle: cx.focus_handle(),
            selected_range: 0..0,
            selection_reversed: false,
            last_layout: None,
            last_bounds: None,
            last_height: None,
            is_selecting: false,
            text_color: color,
            inline_spans,
        }
    }

    /// 누적된 raw 텍스트 — markdown 파싱 등에서 사용.
    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn set_content(&mut self, content: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = content.into();
        if self.selected_range.end > self.content.len() {
            self.selected_range = 0..0;
            self.selection_reversed = false;
        }
        self.inline_spans.clear();
        cx.notify();
    }

    pub fn append(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut s = String::with_capacity(self.content.len() + text.len());
        s.push_str(&self.content);
        s.push_str(text);
        self.content = SharedString::from(s);
        self.inline_spans.clear();
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            return;
        }
        let Some(slice) = self.content.get(self.selected_range.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let (range, reversed) = drag_selection_to(
            self.selected_range.clone(),
            self.selection_reversed,
            offset,
            self.content.len(),
        );
        self.selected_range = range;
        self.selection_reversed = reversed;
        cx.notify();
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>, line_height: Pixels) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        let local = point(position.x - bounds.left(), position.y - bounds.top());
        line.closest_index_for_position(local, line_height)
            .unwrap_or_else(|i| i)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        let idx = self.index_for_mouse_position(event.position, window.line_height());
        if event.modifiers.shift {
            self.select_to(idx, cx);
        } else {
            self.move_to(idx, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            let idx = self.index_for_mouse_position(event.position, window.line_height());
            self.select_to(idx, cx);
        }
    }
}

fn drag_selection_to(
    mut range: Range<usize>,
    mut reversed: bool,
    offset: usize,
    content_len: usize,
) -> (Range<usize>, bool) {
    let offset = offset.min(content_len);
    if reversed {
        range.start = offset;
    } else {
        range.end = offset;
    }
    if range.end < range.start {
        reversed = !reversed;
        range = range.end..range.start;
    }
    (range, reversed)
}

fn text_runs_for_inline(
    len: usize,
    base_font: gpui::Font,
    base_color: u32,
    spans: &[InlineSpan],
) -> Vec<TextRun> {
    let base_color = gpui::rgb(base_color).into();
    if len == 0 {
        return vec![TextRun {
            len: 1,
            font: base_font,
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
    }

    let mut runs = Vec::new();
    let mut cursor = 0;
    for span in spans {
        if span.range.start > cursor {
            runs.push(TextRun {
                len: span.range.start - cursor,
                font: base_font.clone(),
                color: base_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        let (font, color, background_color, underline) = match span.kind {
            InlineKind::Code => (
                font("Menlo"),
                gpui::rgb(0xc0d8e8).into(),
                Some(gpui::rgb(0x263140).into()),
                None,
            ),
            InlineKind::Bold => (base_font.clone().bold(), base_color, None, None),
            InlineKind::Link => (
                base_font.clone(),
                gpui::rgb(0x8bb8ff).into(),
                None,
                Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(gpui::rgb(0x8bb8ff).into()),
                    wavy: false,
                }),
            ),
        };
        runs.push(TextRun {
            len: span.range.end - span.range.start,
            font,
            color,
            background_color,
            underline,
            strikethrough: None,
        });
        cursor = span.range.end;
    }
    if cursor < len {
        runs.push(TextRun {
            len: len - cursor,
            font: base_font,
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}

impl Focusable for SelectableText {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SelectableText {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("SelectableText")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(TextElement {
                entity: cx.entity(),
            })
    }
}

// EntityInputHandler — IME/cursor 등은 ElementInputHandler가 요구. read-only라
// insert/delete는 noop.
impl EntityInputHandler for SelectableText {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        self.content.get(range).map(|s| s.to_string())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.selected_range.clone(),
            reversed: self.selection_reversed,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

// ─── 텍스트 paint element ────────────────────────────────────

struct TextElement {
    entity: Entity<SelectableText>,
}

struct PrepaintState {
    line: Option<WrappedLine>,
    selection: Vec<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let line_height = window.line_height();
        let height = self
            .entity
            .read(cx)
            .last_height
            .unwrap_or(line_height)
            .max(line_height);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = height.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let entity = self.entity.read(cx);
        let raw_content = entity.content.clone();
        let selected = entity.selected_range.clone();
        let color = entity.text_color;
        let inline_spans = entity.inline_spans.clone();
        let _ = entity;

        // 빈 문자열은 공백 1자로 (layout 0 height 방지).
        let content: SharedString = if raw_content.is_empty() {
            " ".into()
        } else {
            raw_content
        };

        let style = window.text_style();
        let line_height = window.line_height();
        let runs = text_runs_for_inline(content.len(), style.font(), color, &inline_spans);
        let font_size = style.font_size.to_pixels(window.rem_size());
        let lines = window
            .text_system()
            .shape_text(content, font_size, &runs, Some(bounds.size.width), None)
            .unwrap_or_default();
        let line = lines.into_iter().next();

        // selection rect (multi-line 인식, ChatInput과 같은 패턴)
        let mut selection = Vec::new();
        if let Some(line) = &line {
            if !selected.is_empty() && selected.end <= line.text.len() {
                if let (Some(start), Some(end)) = (
                    line.position_for_index(selected.start, line_height),
                    line.position_for_index(selected.end, line_height),
                ) {
                    if (start.y - end.y).abs() < line_height / 2. {
                        selection.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + start.x, bounds.top() + start.y),
                                point(
                                    bounds.left() + end.x,
                                    bounds.top() + start.y + line_height,
                                ),
                            ),
                            rgba(0x4f6fff60),
                        ));
                    } else {
                        selection.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + start.x, bounds.top() + start.y),
                                point(bounds.right(), bounds.top() + start.y + line_height),
                            ),
                            rgba(0x4f6fff60),
                        ));
                        selection.push(fill(
                            Bounds::from_corners(
                                point(bounds.left(), bounds.top() + end.y),
                                point(bounds.left() + end.x, bounds.top() + end.y + line_height),
                            ),
                            rgba(0x4f6fff60),
                        ));
                        let mut y = start.y + line_height;
                        while y < end.y {
                            selection.push(fill(
                                Bounds::from_corners(
                                    point(bounds.left(), bounds.top() + y),
                                    point(bounds.right(), bounds.top() + y + line_height),
                                ),
                                rgba(0x4f6fff60),
                            ));
                            y += line_height;
                        }
                    }
                }
            }
        }

        // height 캐시 (다음 frame request_layout)
        let height = line
            .as_ref()
            .map(|l| l.size(line_height).height)
            .unwrap_or(line_height);
        let entity = self.entity.clone();
        entity.update(cx, |this, cx| {
            if this.last_height != Some(height) {
                this.last_height = Some(height);
                cx.notify();
            }
        });

        PrepaintState { line, selection }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.entity.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.entity.clone()),
            cx,
        );
        for sel in prepaint.selection.drain(..) {
            window.paint_quad(sel);
        }
        let Some(line) = prepaint.line.take() else {
            return;
        };
        let _ = line.paint(
            bounds.origin,
            window.line_height(),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
        self.entity.update(cx, |this, _| {
            this.last_layout = Some(line);
            this.last_bounds = Some(bounds);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_selection_extends_forward() {
        let (range, reversed) = drag_selection_to(3..3, false, 8, 20);

        assert_eq!(range, 3..8);
        assert!(!reversed);
    }

    #[test]
    fn drag_selection_crosses_anchor_and_becomes_reversed() {
        let (range, reversed) = drag_selection_to(8..8, false, 3, 20);

        assert_eq!(range, 3..8);
        assert!(reversed);
    }

    #[test]
    fn reversed_drag_moves_selection_start() {
        let (range, reversed) = drag_selection_to(3..8, true, 5, 20);

        assert_eq!(range, 5..8);
        assert!(reversed);
    }

    #[test]
    fn reversed_drag_crosses_back_to_forward() {
        let (range, reversed) = drag_selection_to(3..8, true, 10, 20);

        assert_eq!(range, 8..10);
        assert!(!reversed);
    }

    #[test]
    fn drag_selection_clamps_to_content_len() {
        let (range, reversed) = drag_selection_to(3..3, false, 30, 10);

        assert_eq!(range, 3..10);
        assert!(!reversed);
    }
}
