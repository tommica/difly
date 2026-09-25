//! Large-document editor: full-document cursors, compact line geometry, visible glyphs only.
use crate::{
    diff_view::{GUTTER, LINE_HEIGHT, TEXT_INSET, TEXT_PADDING, split_widths},
    ui::{Colors, Highlight},
};
use difly::model::{self, DiffRows, Hunk};
use eframe::egui::{
    self, Color32, FontId, Id, Key, Pos2, Rect, Sense, Stroke, Vec2,
    text::{CCursor, CCursorRange},
};
use std::{ops::Range, sync::Arc};

const FONT_SIZE: f32 = 15.0;
#[derive(Clone, Debug)]
struct Segment {
    bytes: Range<usize>,
    chars: Range<usize>,
    width: f32,
}
#[derive(Clone, Debug)]
struct Line {
    start: usize,
    segments: Vec<Segment>,
}
struct Geometry {
    old: String,
    new: String,
    options: Options,
    ppp: f32,
    rows: DiffRows,
    old_lines: Vec<Line>,
    new_lines: Vec<Line>,
    offsets: Vec<f32>,
    old_width: f32,
    new_width: f32,
    height: f32,
}
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Options {
    pub width: f32,
    pub split: f32,
    pub wrap: bool,
    pub unified: bool,
    pub dark: bool,
}
#[derive(Default)]
pub(crate) struct VirtualDiff {
    geometry: Option<Arc<Geometry>>,
    follow_cursor: bool,
    preferred_x: Option<f32>,
    preedit: String,
}
#[derive(Default)]
pub(crate) struct Outcome {
    pub changed: bool,
    pub selection_changed: bool,
    pub revert: Option<usize>,
    pub split: Option<f32>,
}

fn char_byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(b, _)| b)
}
fn selected_bytes(text: &str, cursor: CCursorRange) -> Range<usize> {
    let [a, b] = cursor.sorted_cursors();
    char_byte(text, a.index)..char_byte(text, b.index)
}
fn replace(text: &mut String, cursor: &mut CCursorRange, insert: &str) {
    let [a, _] = cursor.sorted_cursors();
    text.replace_range(selected_bytes(text, *cursor), insert);
    *cursor = CCursorRange::one(CCursor::new(a.index + insert.chars().count()));
}
fn move_to(cursor: &mut CCursorRange, index: usize, shift: bool) {
    cursor.primary = CCursor::new(index);
    if !shift {
        cursor.secondary = cursor.primary;
    }
}
fn word_left(text: &str, at: usize) -> usize {
    let chars: Vec<_> = text.chars().take(at).collect();
    let mut i = chars.len();
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i > 0 {
        let word = chars[i - 1].is_alphanumeric() || chars[i - 1] == '_';
        while i > 0
            && !chars[i - 1].is_whitespace()
            && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') == word
        {
            i -= 1;
        }
    }
    i
}
fn word_right(text: &str, at: usize) -> usize {
    let chars: Vec<_> = text.chars().collect();
    let mut i = at.min(chars.len());
    if i < chars.len() {
        let word = chars[i].is_alphanumeric() || chars[i] == '_';
        while i < chars.len()
            && !chars[i].is_whitespace()
            && (chars[i].is_alphanumeric() || chars[i] == '_') == word
        {
            i += 1;
        }
    }
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}
fn advance(fonts: &mut egui::epaint::text::FontsView<'_>, c: char) -> f32 {
    if c == '\r' {
        0.0
    } else if c == '\t' {
        fonts.glyph_width(&FontId::monospace(FONT_SIZE), ' ') * 4.0
    } else {
        fonts.glyph_width(&FontId::monospace(FONT_SIZE), c)
    }
}
fn lines(ui: &egui::Ui, text: &str, width: f32) -> Vec<Line> {
    ui.fonts_mut(|fonts| {
        let mut result = Vec::new();
        let mut byte = 0;
        let mut character = 0;
        for line in text.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let chars: Vec<_> = content
                .char_indices()
                .map(|(b, c)| (b, c, advance(fonts, c)))
                .collect();
            let mut segments = vec![];
            let mut start = 0;
            while start < chars.len() {
                let mut end = start;
                let mut x = 0.0;
                let mut boundary = None;
                while end < chars.len() {
                    if end > start && x + chars[end].2 > width {
                        break;
                    }
                    x += chars[end].2;
                    end += 1;
                    if chars[end - 1].1.is_whitespace() {
                        boundary = Some(end);
                    }
                }
                if end < chars.len()
                    && let Some(b) = boundary
                {
                    end = b;
                }
                let finish = chars.get(end).map_or(content.len(), |c| c.0);
                segments.push(Segment {
                    bytes: byte + chars[start].0..byte + finish,
                    chars: character + start..character + end,
                    width: chars[start..end].iter().map(|c| c.2).sum(),
                });
                start = end;
            }
            if segments.is_empty() {
                segments.push(Segment {
                    bytes: byte..byte,
                    chars: character..character,
                    width: 0.0,
                });
            }
            result.push(Line {
                start: character,
                segments,
            });
            byte += line.len();
            character += line.chars().count();
        }
        if text.is_empty() || text.ends_with('\n') {
            result.push(Line {
                start: character,
                segments: vec![Segment {
                    bytes: byte..byte,
                    chars: character..character,
                    width: 0.0,
                }],
            });
        }
        result
    })
}
impl Geometry {
    fn build(ui: &egui::Ui, old: &str, new: &str, options: Options) -> Self {
        let available = (options.width - GUTTER).max(80.0);
        let [a, b] = if options.unified {
            [available, available]
        } else {
            split_widths(available, options.split)
        };
        let max_width = |w: f32| {
            if options.wrap {
                (w - TEXT_INSET - TEXT_PADDING).max(24.0)
            } else {
                f32::INFINITY
            }
        };
        let old_lines = lines(ui, old, max_width(a));
        let new_lines = lines(ui, new, max_width(b));
        let hunks = model::hunks(old, new);
        let mut rows = model::diff_rows(old, new, &hunks, options.unified);
        let terminal = |text: &str, lines: &[Line]| {
            (text.is_empty() || text.ends_with('\n')).then_some(lines.len() - 1)
        };
        rows.rows
            .push((terminal(old, &old_lines), terminal(new, &new_lines)));
        let mut offsets = vec![0.0];
        for (a, b) in &rows.rows {
            let count = a
                .map(|n| old_lines[n].segments.len())
                .unwrap_or(0)
                .max(b.map(|n| new_lines[n].segments.len()).unwrap_or(0))
                .max(1);
            offsets.push(offsets.last().unwrap() + count as f32 * LINE_HEIGHT);
        }
        let old_max = old_lines
            .iter()
            .flat_map(|l| &l.segments)
            .map(|s| s.width)
            .fold(0.0_f32, f32::max)
            + TEXT_INSET
            + TEXT_PADDING;
        let new_max = new_lines
            .iter()
            .flat_map(|l| &l.segments)
            .map(|s| s.width)
            .fold(0.0_f32, f32::max)
            + TEXT_INSET
            + TEXT_PADDING;
        let (old_width, new_width) = if options.wrap {
            (a, b)
        } else if options.unified {
            let w = available.max(old_max).max(new_max);
            (w, w)
        } else {
            let ratio = a / available;
            let total = available.max(old_max / ratio).max(new_max / (1.0 - ratio));
            (total * ratio, total * (1.0 - ratio))
        };
        let height = offsets.last().unwrap() + LINE_HEIGHT;
        Self {
            old: old.into(),
            new: new.into(),
            options,
            ppp: ui.ctx().pixels_per_point(),
            rows,
            old_lines,
            new_lines,
            offsets,
            old_width,
            new_width,
            height,
        }
    }
    fn side(&self, old: bool) -> (&str, &[Line], &[usize]) {
        if old {
            (&self.old, &self.old_lines, &self.rows.old_y)
        } else {
            (&self.new, &self.new_lines, &self.rows.new_y)
        }
    }
    fn location(&self, index: usize, old: bool) -> (usize, usize) {
        let (_, lines, _) = self.side(old);
        let line = lines
            .partition_point(|l| l.start <= index)
            .saturating_sub(1);
        let segments = &lines[line].segments;
        let segment = segments
            .partition_point(|s| s.chars.start <= index)
            .saturating_sub(1);
        (line, segment)
    }
    fn position(&self, ui: &egui::Ui, index: usize, old: bool) -> Pos2 {
        let (text, lines, map) = self.side(old);
        let (line, segment) = self.location(index, old);
        let s = &lines[line].segments[segment];
        let x = ui.fonts_mut(|f| {
            text[s.bytes.clone()]
                .chars()
                .take(index.saturating_sub(s.chars.start))
                .map(|c| advance(f, c))
                .sum()
        });
        Pos2::new(x, self.offsets[map[line]] + segment as f32 * LINE_HEIGHT)
    }
    fn hit(&self, ui: &egui::Ui, p: Pos2, old: bool) -> usize {
        let (text, lines, map) = self.side(old);
        let row = self
            .offsets
            .partition_point(|&y| y <= p.y)
            .saturating_sub(1)
            .min(self.rows.rows.len() - 1);
        let line = map
            .partition_point(|&r| r <= row)
            .saturating_sub(1)
            .min(lines.len() - 1);
        let segment = ((p.y - self.offsets[map[line]]).max(0.0) / LINE_HEIGHT) as usize;
        let s = &lines[line].segments[segment.min(lines[line].segments.len() - 1)];
        ui.fonts_mut(|fonts| {
            let mut x = 0.0;
            for (i, c) in text[s.bytes.clone()].chars().enumerate() {
                let w = advance(fonts, c);
                if p.x < x + w * 0.5 {
                    return s.chars.start + i;
                }
                x += w;
            }
            s.chars.end
        })
    }
    fn vertical(&self, ui: &egui::Ui, at: usize, old: bool, delta: isize, x: f32) -> usize {
        let (_, lines, map) = self.side(old);
        let (mut line, mut segment) = self.location(at, old);
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                if segment > 0 {
                    segment -= 1;
                } else if line > 0 {
                    line -= 1;
                    segment = lines[line].segments.len() - 1;
                }
            } else if segment + 1 < lines[line].segments.len() {
                segment += 1;
            } else if line + 1 < lines.len() {
                line += 1;
                segment = 0;
            }
        }
        self.hit(
            ui,
            Pos2::new(
                x,
                self.offsets[map[line]] + segment as f32 * LINE_HEIGHT + 1.0,
            ),
            old,
        )
    }
}

impl VirtualDiff {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: Id,
        old: &str,
        text: &mut String,
        hunks: &mut [Hunk],
        highlight: &mut Highlight,
        extension: &str,
        options: Options,
        jump: Option<usize>,
    ) -> Outcome {
        if self.geometry.as_ref().is_none_or(|g| {
            g.old != old
                || g.new != *text
                || g.options != options
                || g.ppp != ui.ctx().pixels_per_point()
        }) {
            self.geometry = Some(Arc::new(Geometry::build(ui, old, text, options)));
        }
        let g = self.geometry.as_ref().unwrap().clone();
        let old_doc = highlight.document("old", old, extension, options.dark);
        let new_doc = highlight.document("working", text, extension, options.dark);
        let colors = Colors::new(options.dark);
        let mut out = Outcome::default();
        let mut scroll = if options.wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };
        scroll = scroll
            .id_salt(("virtual-diff", id, options.unified, options.wrap))
            .auto_shrink([false, false]);
        if let Some(range) = jump.and_then(|h| g.rows.hunk_rows.get(h)) {
            scroll = scroll.vertical_scroll_offset(g.offsets[range.start]);
        }
        scroll.show(ui, |ui| {
            let origin = ui.cursor().min;
            let width = GUTTER
                + if options.unified {
                    g.new_width
                } else {
                    g.old_width + g.new_width
                };
            let height = g.height.max(ui.clip_rect().height() - 28.0);
            ui.allocate_space(Vec2::new(width, height + 28.0));
            let control_x = origin.x + if options.unified { 0.0 } else { g.old_width };
            let old_origin = origin + Vec2::new(if options.unified { GUTTER } else { 0.0 }, 28.0);
            let new_origin = if options.unified {
                old_origin
            } else {
                origin + Vec2::new(g.old_width + GUTTER, 28.0)
            };
            ui.painter().rect_filled(
                Rect::from_min_size(
                    Pos2::new(control_x, origin.y),
                    Vec2::new(GUTTER, height + 28.0),
                ),
                0.0,
                colors.panel,
            );
            if !options.unified {
                let rect = Rect::from_min_size(
                    Pos2::new(control_x, origin.y),
                    Vec2::new(GUTTER, height + 28.0),
                );
                let response = ui
                    .interact(rect, id.with("divider"), Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                for dx in [-4.0, 0.0, 4.0] {
                    ui.painter().vline(
                        rect.center().x + dx,
                        origin.y + 10.0..=origin.y + 18.0,
                        Stroke::new(1.0_f32, colors.muted),
                    );
                }
                if response.dragged() {
                    out.split = Some(
                        (options.split
                            + ui.input(|i| i.pointer.delta().x) / (options.width - GUTTER))
                            .clamp(0.1, 0.9),
                    );
                }
                if response.double_clicked() {
                    out.split = Some(0.5);
                }
            }
            ui.painter().text(
                old_origin + Vec2::new(8.0, -16.0),
                egui::Align2::LEFT_CENTER,
                if options.unified {
                    "WORKING FILE · removed lines are read-only"
                } else {
                    "HEAD · read-only"
                },
                FontId::proportional(11.0),
                colors.muted,
            );
            if !options.unified {
                ui.painter().text(
                    new_origin + Vec2::new(8.0, -16.0),
                    egui::Align2::LEFT_CENTER,
                    "WORKING FILE · editable",
                    FontId::proportional(11.0),
                    colors.muted,
                );
            }
            let visible_start = (ui.clip_rect().top() - new_origin.y).max(0.0);
            let visible_end = ui.clip_rect().bottom() - new_origin.y;
            let first = g
                .offsets
                .partition_point(|&y| y < visible_start)
                .saturating_sub(1);
            let last = g
                .offsets
                .partition_point(|&y| y <= visible_end)
                .min(g.rows.rows.len());
            for (h, range) in g.rows.hunk_rows.iter().enumerate() {
                let top = new_origin.y + g.offsets[range.start];
                let bottom = new_origin.y + g.offsets[range.end];
                if bottom < ui.clip_rect().top() || top > ui.clip_rect().bottom() {
                    continue;
                }
                if let Some(hunk) = hunks.get_mut(h) {
                    if hunk.selected {
                        ui.painter().rect_filled(
                            Rect::from_min_max(
                                Pos2::new(control_x, top),
                                Pos2::new(control_x + GUTTER, bottom),
                            ),
                            0.0,
                            colors.included,
                        );
                    }
                    // Keep controls reachable when the beginning of a large hunk scrolls away.
                    let y = top.max(ui.clip_rect().top()).min(bottom - LINE_HEIGHT);
                    if ui
                        .put(
                            Rect::from_min_size(
                                Pos2::new(control_x + 34.0, y),
                                Vec2::new(25.0, LINE_HEIGHT),
                            ),
                            egui::Checkbox::without_text(&mut hunk.selected),
                        )
                        .changed()
                    {
                        hunk.review = false;
                        out.selection_changed = true;
                    }
                    if ui
                        .put(
                            Rect::from_min_size(
                                Pos2::new(control_x + 3.0, y),
                                Vec2::new(28.0, LINE_HEIGHT),
                            ),
                            egui::Button::new(">>").frame(false),
                        )
                        .on_hover_text("Revert this hunk")
                        .clicked()
                    {
                        out.revert = Some(h);
                    }
                    if hunk.review {
                        ui.painter().circle_filled(
                            Pos2::new(control_x + 60.0, y + 4.0),
                            3.0,
                            colors.warn,
                        );
                    }
                }
            }
            for row in first..last {
                let (a, b) = g.rows.rows[row];
                let changed = g.rows.hunk_rows.iter().any(|r| r.contains(&row));
                for is_old in [true, false] {
                    if options.unified && is_old && b.is_some() {
                        continue;
                    }
                    let line = if is_old { a } else { b };
                    let origin = if is_old { old_origin } else { new_origin };
                    let pane = if is_old { g.old_width } else { g.new_width };
                    let row_rect = Rect::from_min_size(
                        origin + Vec2::new(0.0, g.offsets[row]),
                        Vec2::new(pane, g.offsets[row + 1] - g.offsets[row]),
                    );
                    if changed && line.is_some() {
                        ui.painter().rect_filled(
                            row_rect,
                            0.0,
                            if is_old { colors.del } else { colors.add },
                        );
                    }
                    let Some(line) = line else { continue };
                    ui.painter().text(
                        origin + Vec2::new(37.0, g.offsets[row] + LINE_HEIGHT * 0.5),
                        egui::Align2::RIGHT_CENTER,
                        (line + 1).to_string(),
                        FontId::monospace(FONT_SIZE),
                        colors.muted,
                    );
                }
            }
            // Input and selection use full-document character positions, independent of rendered rows.
            for is_old in [true, false] {
                if is_old && options.unified {
                    continue;
                }
                let editor_id = if is_old { id.with("old-virtual") } else { id };
                let origin = if is_old { old_origin } else { new_origin };
                let pane = if is_old { g.old_width } else { g.new_width };
                let rect = Rect::from_min_size(
                    origin + Vec2::new(TEXT_INSET, 0.0),
                    Vec2::new(pane - TEXT_INSET - TEXT_PADDING, height),
                );
                let response = ui
                    .interact(
                        rect.intersect(ui.clip_rect()),
                        editor_id,
                        Sense::click_and_drag(),
                    )
                    .on_hover_cursor(egui::CursorIcon::Text);
                if !is_old && response.hovered() && ui.is_enabled() {
                    ui.output_mut(|o| o.mutable_text_under_cursor = true);
                }
                if response.has_focus() && ui.is_enabled() {
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            editor_id,
                            egui::EventFilter {
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                tab: true,
                                ..Default::default()
                            },
                        )
                    });
                }
                let mut state =
                    egui::text_edit::TextEditState::load(ui.ctx(), editor_id).unwrap_or_default();
                let mut cursor = state.cursor.char_range().unwrap_or_default();
                let content = if is_old { old } else { text.as_str() };
                let count = content.chars().count();
                cursor.primary.index = cursor.primary.index.min(count);
                cursor.secondary.index = cursor.secondary.index.min(count);
                let mut follow = false;
                let pressed =
                    response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed());
                if (pressed || response.clicked() || response.dragged())
                    && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
                {
                    response.request_focus();
                    let pos = g.hit(
                        ui,
                        Pos2::new(pointer.x - rect.left(), pointer.y - origin.y),
                        is_old,
                    );
                    let extend =
                        (!pressed && response.dragged()) || ui.input(|i| i.modifiers.shift);
                    move_to(&mut cursor, pos, extend);
                    if response.double_clicked() {
                        cursor = CCursorRange::two(
                            CCursor::new(word_left(content, pos)),
                            CCursor::new(word_right(content, pos)),
                        );
                    }
                    self.preferred_x = None;
                    if response.dragged() {
                        ui.scroll_to_rect(Rect::from_min_size(pointer, Vec2::splat(2.0)), None);
                    }
                }
                if response.has_focus() && ui.is_enabled() {
                    let events = ui.input(|i| i.events.clone());
                    let before = (cursor, text.clone());
                    let mut history = state.undoer();
                    if !is_old {
                        history.feed_state(ui.input(|i| i.time), &before);
                    }
                    for event in events {
                        let content = if is_old { old } else { text.as_str() };
                        let length = content.chars().count();
                        match event {
                            egui::Event::Copy | egui::Event::Cut => {
                                ui.ctx()
                                    .copy_text(content[selected_bytes(content, cursor)].into());
                                if !is_old && matches!(event, egui::Event::Cut) {
                                    replace(text, &mut cursor, "");
                                    follow = true;
                                }
                            }
                            egui::Event::Text(insert) | egui::Event::Paste(insert) if !is_old => {
                                replace(text, &mut cursor, &insert);
                                follow = true;
                            }
                            egui::Event::Ime(egui::ImeEvent::Disabled) => {
                                self.preedit.clear();
                            }
                            egui::Event::Ime(egui::ImeEvent::Preedit(value)) if !is_old => {
                                self.preedit = value;
                            }
                            egui::Event::Ime(egui::ImeEvent::Commit(value)) if !is_old => {
                                replace(text, &mut cursor, &value);
                                self.preedit.clear();
                                follow = true;
                            }
                            egui::Event::Key {
                                key,
                                pressed: true,
                                modifiers,
                                ..
                            } => {
                                let at = cursor.primary.index;
                                let word = modifiers.alt || (modifiers.ctrl && !modifiers.mac_cmd);
                                let (line, segment) = g.location(at, is_old);
                                let (_, lines, _) = g.side(is_old);
                                let boundary = &lines[line].segments[segment];
                                let mut target = None;
                                match key {
                                    Key::A if modifiers.command => {
                                        cursor = CCursorRange::two(
                                            CCursor::new(0),
                                            CCursor::new(length),
                                        );
                                    }
                                    Key::Z if modifiers.command && !is_old => {
                                        let current = (cursor, text.clone());
                                        let result = if modifiers.shift {
                                            history.redo(&current)
                                        } else {
                                            history.undo(&current)
                                        };
                                        if let Some((c, value)) = result.cloned() {
                                            *text = value;
                                            cursor = c;
                                            follow = true;
                                        }
                                    }
                                    Key::ArrowLeft => {
                                        target = Some(if !modifiers.shift && !cursor.is_empty() {
                                            cursor.sorted_cursors()[0].index
                                        } else if modifiers.mac_cmd {
                                            boundary.chars.start
                                        } else if word {
                                            word_left(content, at)
                                        } else {
                                            at.saturating_sub(1)
                                        });
                                    }
                                    Key::ArrowRight => {
                                        target = Some(if !modifiers.shift && !cursor.is_empty() {
                                            cursor.sorted_cursors()[1].index
                                        } else if modifiers.mac_cmd {
                                            boundary.chars.end
                                        } else if word {
                                            word_right(content, at)
                                        } else {
                                            (at + 1).min(length)
                                        });
                                    }
                                    Key::Home => {
                                        target = Some(if modifiers.command {
                                            0
                                        } else {
                                            boundary.chars.start
                                        });
                                    }
                                    Key::End => {
                                        target = Some(if modifiers.command {
                                            length
                                        } else {
                                            boundary.chars.end
                                        });
                                    }
                                    Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
                                        if modifiers.mac_cmd {
                                            target =
                                                Some(if key == Key::ArrowUp { 0 } else { length });
                                        } else {
                                            let x = *self.preferred_x.get_or_insert_with(|| {
                                                g.position(ui, at, is_old).x
                                            });
                                            let page = if matches!(key, Key::PageUp | Key::PageDown)
                                            {
                                                (ui.clip_rect().height() / LINE_HEIGHT).max(1.0)
                                                    as isize
                                            } else {
                                                1
                                            };
                                            target = Some(g.vertical(
                                                ui,
                                                at,
                                                is_old,
                                                if matches!(key, Key::ArrowUp | Key::PageUp) {
                                                    -page
                                                } else {
                                                    page
                                                },
                                                x,
                                            ));
                                        }
                                    }
                                    Key::Backspace | Key::Delete if !is_old => {
                                        if cursor.is_empty() {
                                            cursor.secondary.index = if key == Key::Backspace {
                                                if word {
                                                    word_left(content, at)
                                                } else {
                                                    at.saturating_sub(1)
                                                }
                                            } else if word {
                                                word_right(content, at)
                                            } else {
                                                (at + 1).min(length)
                                            };
                                        }
                                        replace(text, &mut cursor, "");
                                        follow = true;
                                    }
                                    Key::Enter if !is_old && !modifiers.command => {
                                        replace(text, &mut cursor, "\n");
                                        follow = true;
                                    }
                                    Key::Tab if !is_old && !modifiers.shift => {
                                        replace(text, &mut cursor, "\t");
                                        follow = true;
                                    }
                                    _ => {}
                                }
                                if let Some(target) = target {
                                    move_to(&mut cursor, target, modifiers.shift);
                                    follow = true;
                                    if !matches!(
                                        key,
                                        Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown
                                    ) {
                                        self.preferred_x = None;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if !is_old && *text != before.1 {
                        self.preferred_x = None;
                        out.changed = true;
                        self.follow_cursor = true;
                        ui.ctx().request_repaint();
                    }
                    if !is_old {
                        history.feed_state(ui.input(|i| i.time), &(cursor, text.clone()));
                    }
                    state.set_undoer(history);
                }
                let [start, end] = cursor.sorted_cursors();
                let (_, lines, map) = g.side(is_old);
                for row in first..last {
                    let line = if is_old {
                        g.rows.rows[row].0
                    } else {
                        g.rows.rows[row].1
                    };
                    let Some(line) = line else { continue };
                    for (i, segment) in lines[line].segments.iter().enumerate() {
                        let y = origin.y + g.offsets[map[line]] + i as f32 * LINE_HEIGHT;
                        if y + LINE_HEIGHT < ui.clip_rect().top() || y > ui.clip_rect().bottom() {
                            continue;
                        }
                        if start.index < segment.chars.end && end.index > segment.chars.start {
                            let left = g
                                .position(ui, start.index.max(segment.chars.start), is_old)
                                .x;
                            let right = if end.index >= segment.chars.end {
                                segment.width
                            } else {
                                g.position(ui, end.index, is_old).x
                            };
                            ui.painter().rect_filled(
                                Rect::from_min_size(
                                    Pos2::new(rect.left() + left, y),
                                    Vec2::new((right - left).max(2.0), LINE_HEIGHT),
                                ),
                                0.0,
                                ui.visuals().selection.bg_fill,
                            );
                        }
                    }
                }
                if response.has_focus() && ui.is_enabled() {
                    let pos = g.position(ui, cursor.primary.index, is_old);
                    let caret = Rect::from_min_size(
                        Pos2::new(rect.left() + pos.x, origin.y + pos.y),
                        Vec2::new(1.0, LINE_HEIGHT),
                    );
                    if !out.changed && (follow || self.follow_cursor) {
                        ui.scroll_to_rect(caret.expand(8.0), None);
                        self.follow_cursor = false;
                    }
                    ui.painter().vline(
                        caret.left(),
                        caret.y_range(),
                        Stroke::new(1.0_f32, colors.accent),
                    );
                    if !is_old {
                        ui.output_mut(|o| {
                            o.ime = Some(egui::output::IMEOutput {
                                rect,
                                cursor_rect: caret,
                            })
                        });
                        if !self.preedit.is_empty() {
                            ui.painter().text(
                                caret.left_top(),
                                egui::Align2::LEFT_TOP,
                                &self.preedit,
                                FontId::monospace(FONT_SIZE),
                                colors.accent,
                            );
                        }
                    }
                }
                state.cursor.set_char_range(Some(cursor));
                state.store(ui.ctx(), editor_id);
            }
            // Glyph meshes are allocated only for visible visual segments.
            for row in first..last {
                let (a, b) = g.rows.rows[row];
                for (is_old, line) in [(true, a), (false, b)] {
                    if options.unified && is_old && b.is_some() {
                        continue;
                    }
                    let Some(line) = line else { continue };
                    let (text, lines, _) = g.side(is_old);
                    let origin = if is_old { old_origin } else { new_origin };
                    let pane = if is_old { g.old_width } else { g.new_width };
                    let clip = ui.clip_rect().intersect(Rect::from_min_size(
                        origin + Vec2::new(TEXT_INSET, 0.0),
                        Vec2::new(pane - TEXT_INSET - TEXT_PADDING, height),
                    ));
                    for (i, s) in lines[line].segments.iter().enumerate() {
                        let y = origin.y + g.offsets[row] + i as f32 * LINE_HEIGHT;
                        if y + LINE_HEIGHT < clip.top() || y > clip.bottom() {
                            continue;
                        }
                        let mut bytes = s.bytes.clone();
                        let mut x = 0.0;
                        // Also bound meshes for a single very long unwrapped line.
                        ui.fonts_mut(|fonts| {
                            let mut started = false;
                            let mut right = 0.0;
                            bytes.end = bytes.start;
                            for (b, c) in text[s.bytes.clone()].char_indices() {
                                let w = advance(fonts, c);
                                if !started
                                    && origin.x + TEXT_INSET + right + w >= clip.left() - 64.0
                                {
                                    bytes.start = s.bytes.start + b;
                                    x = right;
                                    started = true;
                                }
                                right += w;
                                if started {
                                    bytes.end = s.bytes.start + b + c.len_utf8();
                                }
                                if started && origin.x + TEXT_INSET + right > clip.right() + 64.0 {
                                    break;
                                }
                            }
                        });
                        let doc = if is_old { &old_doc } else { &new_doc };
                        let galley = ui.fonts_mut(|f| f.layout_job(doc.job(bytes)));
                        let ppp = ui.ctx().pixels_per_point();
                        let pos = Pos2::new(
                            ((origin.x + TEXT_INSET + x) * ppp).round() / ppp,
                            ((y + (LINE_HEIGHT - galley.size().y) * 0.5) * ppp).round() / ppp,
                        );
                        ui.painter()
                            .with_clip_rect(clip)
                            .galley(pos, galley, Color32::WHITE);
                    }
                }
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(width: f32, wrap: bool, unified: bool) -> Options {
        Options {
            width,
            wrap,
            unified,
            split: 0.5,
            dark: false,
        }
    }
    #[test]
    fn wrapped_geometry_keeps_logical_lines_aligned_and_unicode_positions_round_trip() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for unified in [false, true] {
                    let old = "short\nmatching\n";
                    let new = format!("{}\nmatching\n", "åβ hello\t ".repeat(50));
                    let g = Geometry::build(ui, old, &new, options(400.0, true, unified));
                    assert!(g.new_lines[0].segments.len() > 2);
                    assert_eq!(g.rows.old_y[1], g.rows.new_y[1]);
                    for at in [0, 1, 2, 5, 13, new.chars().count()] {
                        let p = g.position(ui, at, false);
                        assert_eq!(g.hit(ui, p + Vec2::new(0.0, 1.0), false), at);
                    }
                    assert_eq!(g.new, new);
                }
            });
        });
    }
    #[allow(clippy::too_many_arguments)]
    fn draw(
        ctx: &egui::Context,
        view: &mut VirtualDiff,
        high: &mut Highlight,
        old: &str,
        text: &mut String,
        events: Vec<egui::Event>,
        time: f64,
        wrap: bool,
    ) -> (egui::FullOutput, Outcome) {
        let mut result = Outcome::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 500.0))),
                events,
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                crate::ui::map_redo_shortcut(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut hunks = model::hunks(old, text);
                    result = view.show(
                        ui,
                        Id::new("virtual-test"),
                        old,
                        text,
                        &mut hunks,
                        high,
                        "rs",
                        options(ui.available_width(), wrap, false),
                        None,
                    );
                });
            },
        );
        (output, result)
    }
    fn key(key: Key, command: bool, shift: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command,
                shift,
                ..Default::default()
            },
        }
    }
    #[test]
    fn meshes_are_bounded_by_viewport_for_large_files_and_single_long_lines() {
        for wrap in [false, true] {
            for original in [
                (0..4000)
                    .map(|n| format!("let value_{n} = {n};\n"))
                    .collect::<String>(),
                "x".repeat(200_000),
            ] {
                let ctx = egui::Context::default();
                let mut view = VirtualDiff::default();
                let mut high = Highlight::new();
                let mut text = original.clone();
                let (output, _) = draw(
                    &ctx,
                    &mut view,
                    &mut high,
                    &original,
                    &mut text,
                    vec![],
                    0.0,
                    wrap,
                );
                let glyphs: usize = output
                    .shapes
                    .iter()
                    .filter_map(|s| {
                        if let egui::Shape::Text(t) = &s.shape {
                            Some(t.galley.rows.iter().map(|r| r.glyphs.len()).sum::<usize>())
                        } else {
                            None
                        }
                    })
                    .sum();
                assert!(glyphs < 10_000, "off-screen glyphs retained: {glyphs}");
                assert_eq!(text, original);
            }
        }
    }
    #[test]
    fn full_document_selection_paste_undo_redo_and_unicode_editing() {
        let ctx = egui::Context::default();
        let mut view = VirtualDiff::default();
        let mut high = Highlight::new();
        let original = "original line\n".repeat(1000);
        let mut text = original.clone();
        let id = Id::new("virtual-test");
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![],
            0.0,
            true,
        );
        ctx.memory_mut(|m| m.request_focus(id));
        let (_, out) = draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![
                key(Key::A, true, false),
                egui::Event::Paste("αβ\nnew".into()),
            ],
            1.0,
            true,
        );
        assert!(out.changed);
        assert_eq!(text, "αβ\nnew");
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![key(Key::Z, true, false)],
            1.1,
            true,
        );
        assert_eq!(text, original);
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![key(Key::R, true, false)],
            1.2,
            true,
        );
        assert_eq!(text, "αβ\nnew");
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![
                key(Key::Home, true, false),
                key(Key::ArrowRight, false, false),
                key(Key::Delete, false, false),
            ],
            2.0,
            true,
        );
        assert_eq!(text, "α\nnew");
    }
    #[test]
    fn select_all_copy_includes_offscreen_text() {
        let ctx = egui::Context::default();
        let mut view = VirtualDiff::default();
        let mut high = Highlight::new();
        let original = "copy this whole document\n".repeat(1000);
        let mut text = original.clone();
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![],
            0.0,
            true,
        );
        ctx.memory_mut(|m| m.request_focus(Id::new("virtual-test")));
        let (output, _) = draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![key(Key::A, true, false), egui::Event::Copy],
            1.0,
            true,
        );
        assert!(
            output
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c,egui::OutputCommand::CopyText(s) if s==&original))
        );
    }
    #[test]
    fn wrapping_and_view_changes_preserve_global_cursor_and_end_navigation() {
        let ctx = egui::Context::default();
        let mut view = VirtualDiff::default();
        let mut high = Highlight::new();
        let original = "long text and more text\n".repeat(1000);
        let mut text = original.clone();
        let id = Id::new("virtual-test");
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![],
            0.0,
            true,
        );
        ctx.memory_mut(|m| m.request_focus(id));
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![key(Key::End, true, false)],
            1.0,
            true,
        );
        assert_eq!(
            egui::text_edit::TextEditState::load(&ctx, id)
                .unwrap()
                .cursor
                .char_range()
                .unwrap()
                .primary
                .index,
            text.chars().count()
        );
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![egui::Event::Text("end!".into())],
            2.0,
            false,
        );
        assert!(text.ends_with("end!"));
        assert!(view.geometry.as_ref().unwrap().new.len() > 20_000);
    }
    #[test]
    fn disabled_editor_cannot_modify_document_during_commit() {
        let ctx = egui::Context::default();
        let mut view = VirtualDiff::default();
        let mut high = Highlight::new();
        let original = "unchanged\n".repeat(1000);
        let mut text = original.clone();
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![],
            0.0,
            true,
        );
        ctx.memory_mut(|m| m.request_focus(Id::new("virtual-test")));
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Text("must not insert".into())],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(false, |ui| {
                        let result = view.show(
                            ui,
                            Id::new("virtual-test"),
                            &original,
                            &mut text,
                            &mut [],
                            &mut high,
                            "txt",
                            options(800.0, true, false),
                            None,
                        );
                        assert!(!result.changed);
                    });
                });
            },
        );
        assert_eq!(text, original);
    }
    #[test]
    fn mac_end_navigation_and_multiline_input_in_one_frame_keep_event_order() {
        let ctx = egui::Context::default();
        let mut view = VirtualDiff::default();
        let mut high = Highlight::new();
        let original = "line\n".repeat(1000);
        let mut text = original.clone();
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![],
            0.0,
            true,
        );
        ctx.memory_mut(|m| m.request_focus(Id::new("virtual-test")));
        let end = egui::Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                mac_cmd: true,
                ..Default::default()
            },
        };
        draw(
            &ctx,
            &mut view,
            &mut high,
            &original,
            &mut text,
            vec![
                end,
                key(Key::Enter, false, false),
                egui::Event::Text("// QA".into()),
                key(Key::Enter, false, false),
            ],
            1.0,
            true,
        );
        assert_eq!(text, format!("{original}\n// QA\n"));
    }
}
