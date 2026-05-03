//! Read-only selectable text — multi-line wrap + character-level selection.
//! shape_text(wrap_width)로 multi-line, height auto-grow.

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgba, App, Bounds, ClipboardItem,
    Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    ParentElement, Pixels, Point, Render, SharedString, Style, Styled, TextRun, UTF16Selection,
    Window, WrappedLine,
};

actions!(selectable_text, [Copy, SelectAll]);

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
        cx.notify();
    }

    pub fn append(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut s = String::with_capacity(self.content.len() + text.len());
        s.push_str(&self.content);
        s.push_str(text);
        self.content = SharedString::from(s);
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
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
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
        let _ = entity;

        // 빈 문자열은 공백 1자로 (layout 0 height 방지).
        let content: SharedString = if raw_content.is_empty() {
            " ".into()
        } else {
            raw_content
        };

        let style = window.text_style();
        let line_height = window.line_height();
        let run = TextRun {
            len: content.len(),
            font: style.font(),
            color: gpui::rgb(color).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let lines = window
            .text_system()
            .shape_text(content, font_size, &[run], Some(bounds.size.width), None)
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
