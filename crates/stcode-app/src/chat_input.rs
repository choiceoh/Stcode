//! 채팅 입력 — read-write 단일 라인 텍스트 인풋. Zed gpui examples/input.rs를
//! 트리밍해 IME(한글 조합) + selection + 클립보드를 살리고 single-line으로 단순화.
//!
//! Enter는 외부에서 KeyBinding으로 잡아서 send 액션을 별도 처리한다.

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, App, Bounds, ClipboardItem,
    Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, GlobalElementId, InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels, Point,
    SharedString, Style, Styled, TextRun, UTF16Selection, Window, WrappedLine,
};

actions!(
    chat_input,
    [
        Submit,
        Backspace,
        Delete,
        Left,
        Right,
        Home,
        End,
        SelectLeft,
        SelectRight,
        SelectAll,
        Copy,
        Paste,
        Cut,
    ]
);

/// 메인에서 한 번 — 키바인딩 등록.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Submit, Some("ChatInput")),
        KeyBinding::new("backspace", Backspace, Some("ChatInput")),
        KeyBinding::new("delete", Delete, Some("ChatInput")),
        KeyBinding::new("left", Left, Some("ChatInput")),
        KeyBinding::new("right", Right, Some("ChatInput")),
        KeyBinding::new("home", Home, Some("ChatInput")),
        KeyBinding::new("end", End, Some("ChatInput")),
        KeyBinding::new("shift-left", SelectLeft, Some("ChatInput")),
        KeyBinding::new("shift-right", SelectRight, Some("ChatInput")),
        KeyBinding::new("cmd-a", SelectAll, Some("ChatInput")),
        KeyBinding::new("cmd-c", Copy, Some("ChatInput")),
        KeyBinding::new("cmd-v", Paste, Some("ChatInput")),
        KeyBinding::new("cmd-x", Cut, Some("ChatInput")),
    ]);
}

pub struct ChatInput {
    content: SharedString,
    placeholder: SharedString,
    focus_handle: FocusHandle,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<WrappedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    last_height: Option<Pixels>,
    is_selecting: bool,
    text_color: u32,
    placeholder_color: u32,
}

impl ChatInput {
    pub fn new(placeholder: impl Into<SharedString>, fg: u32, muted: u32, cx: &mut Context<Self>) -> Self {
        Self {
            content: SharedString::default(),
            placeholder: placeholder.into(),
            focus_handle: cx.focus_handle(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            last_height: None,
            is_selecting: false,
            text_color: fg,
            placeholder_color: muted,
        }
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.content = SharedString::default();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    /// 초기값 set — 설정 모달 같이 기존 값을 prefill 해야 할 때.
    pub fn set_content(&mut self, content: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = content.into();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
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

    /// UTF16 offset → UTF8 byte offset. IME가 주는 range는 UTF16 단위라
    /// 한글 같은 multi-byte는 그대로 byte index로 쓰면 char boundary 어긋나 panic.
    fn offset_from_utf16(&self, target: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= target {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, target: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf8 >= target {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end)
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    fn previous_grapheme_boundary(&self, offset: usize) -> usize {
        // ASCII는 단일 byte, 한글 등은 multi-byte. char_indices로 안전한 boundary.
        self.content[..offset]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_grapheme_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| offset + i)
            .unwrap_or(self.content.len())
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_grapheme_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_grapheme_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_grapheme_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_grapheme_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_grapheme_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_grapheme_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            return;
        }
        if let Some(slice) = self.content.get(self.selected_range.clone()) {
            cx.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.copy(&Copy, window, cx);
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        let line_height = window.line_height();
        let idx = self.index_for_mouse_position(event.position, line_height);
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
            let line_height = window.line_height();
            self.select_to(self.index_for_mouse_position(event.position, line_height), cx);
        }
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
}

impl Focusable for ChatInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui::Render for ChatInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("ChatInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(InputElement {
                entity: cx.entity(),
            })
    }
}

impl EntityInputHandler for ChatInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        self.content.get(range).map(|s| s.to_string())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let mut new_content =
            String::with_capacity(self.content.len() - (range.end - range.start) + text.len());
        new_content.push_str(&self.content[..range.start]);
        new_content.push_str(text);
        new_content.push_str(&self.content[range.end..]);
        self.content = new_content.into();
        let new_pos = range.start + text.len();
        self.selected_range = new_pos..new_pos;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_marked_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let mut new_content =
            String::with_capacity(self.content.len() - (range.end - range.start) + text.len());
        new_content.push_str(&self.content[..range.start]);
        new_content.push_str(text);
        new_content.push_str(&self.content[range.end..]);
        self.content = new_content.into();
        // marked_range는 새로 들어온 텍스트 안의 utf16 인덱스 — utf8로 환산.
        // 임시 string에서 자체 변환.
        let new_marked_utf8 = new_marked_range_utf16.as_ref().map(|r| {
            let mut utf8 = 0;
            let mut utf16 = 0;
            let mut start = 0;
            for (i, ch) in text.char_indices() {
                if utf16 == r.start {
                    start = i;
                }
                if utf16 >= r.end {
                    return (range.start + start)..(range.start + i);
                }
                utf16 += ch.len_utf16();
                utf8 += ch.len_utf8();
            }
            let _ = utf8;
            (range.start + start)..(range.start + text.len())
        });
        self.marked_range = new_marked_utf8;
        let pos = self
            .marked_range
            .as_ref()
            .map(|r| r.end)
            .unwrap_or(range.start + text.len());
        self.selected_range = pos..pos;
        self.selection_reversed = false;
        cx.notify();
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

// ─── 인풋 element ────────────────────────────────────────────

struct InputElement {
    entity: Entity<ChatInput>,
}

struct PrepaintState {
    line: Option<WrappedLine>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
}

impl PrepaintState {
    fn with_height(self, entity: Entity<ChatInput>, height: Pixels, cx: &mut App) -> Self {
        entity.update(cx, |this, cx| {
            if this.last_height != Some(height) {
                this.last_height = Some(height);
                cx.notify();
            }
        });
        self
    }
}

fn raw_safe_len(line: &WrappedLine) -> usize {
    line.text.len()
}

impl IntoElement for InputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for InputElement {
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
        // 이전 frame의 height 캐시. 첫 frame엔 단일 line 가정.
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
        let raw = entity.content.clone();
        let placeholder = entity.placeholder.clone();
        let cursor_pos = entity.cursor_offset();
        let selected_range = entity.selected_range.clone();
        let color = entity.text_color;
        let pcolor = entity.placeholder_color;
        let _ = entity;

        let (display, color_use, is_placeholder) = if raw.is_empty() {
            (placeholder, pcolor, true)
        } else {
            (raw, color, false)
        };

        let style = window.text_style();
        let line_height = window.line_height();
        let run = TextRun {
            len: display.len(),
            font: style.font(),
            color: rgb(color_use).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let lines = window
            .text_system()
            .shape_text(
                display,
                font_size,
                &[run],
                Some(bounds.size.width),
                None,
            )
            .unwrap_or_default();
        // shape_text는 줄(\n) 단위로 multiple WrappedLine 반환. \n 미사용이라 1개.
        let line = lines.into_iter().next();

        let cursor = match (&line, is_placeholder) {
            (Some(line), false) => line
                .position_for_index(cursor_pos, line_height)
                .map(|pos| {
                    fill(
                        Bounds::new(
                            point(bounds.left() + pos.x, bounds.top() + pos.y),
                            gpui::size(px(2.), line_height),
                        ),
                        rgb(color),
                    )
                }),
            _ => None,
        };

        // selection rect: range가 같은 wrap 줄 안이면 단일 rect, 여러 줄에 걸치면
        // 시작~끝 사이의 모든 wrap 줄 별 rect (단순화 — 첫 줄은 start부터 줄끝,
        // 중간 줄은 통째로, 마지막 줄은 줄시작부터 end).
        let mut selection = Vec::new();
        if let Some(line) = &line {
            if !selected_range.is_empty()
                && selected_range.end <= raw_safe_len(line) {
                if let (Some(start), Some(end)) = (
                    line.position_for_index(selected_range.start, line_height),
                    line.position_for_index(selected_range.end, line_height),
                ) {
                    if (start.y - end.y).abs() < line_height / 2. {
                        // 같은 wrap 줄
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
                        // 시작 줄: start ~ bounds.right
                        selection.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + start.x, bounds.top() + start.y),
                                point(bounds.right(), bounds.top() + start.y + line_height),
                            ),
                            rgba(0x4f6fff60),
                        ));
                        // 마지막 줄: bounds.left ~ end
                        selection.push(fill(
                            Bounds::from_corners(
                                point(bounds.left(), bounds.top() + end.y),
                                point(bounds.left() + end.x, bounds.top() + end.y + line_height),
                            ),
                            rgba(0x4f6fff60),
                        ));
                        // 사이 줄들 — full width (단순화: 한 줄당 1 rect, line_height 단위)
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

        // 다음 frame request_layout에서 사용할 height 캐시.
        let height = line
            .as_ref()
            .map(|l| l.size(line_height).height)
            .unwrap_or(line_height);
        PrepaintState {
            line,
            cursor,
            selection,
        }
        .with_height(self.entity.clone(), height, cx)
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
        let Some(line) = prepaint.line.take() else {
            return;
        };
        for sel in prepaint.selection.drain(..) {
            window.paint_quad(sel);
        }
        let _ = line.paint(
            bounds.origin,
            window.line_height(),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        self.entity.update(cx, |this, _| {
            this.last_layout = Some(line);
            this.last_bounds = Some(bounds);
        });
    }
}
