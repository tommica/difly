//! Paint behind the toolkit's editor using its current display map; never alter source text.
use gpui_kit::component::input::EditorState;
use gpui_kit::{prelude::*, *};
use std::ops::Range;

pub const CODE_SIZE: f32 = 16.;
pub const ROW_HEIGHT: f32 = 24.;

pub fn vertical_range(editor: &EditorState, lines: &Range<usize>) -> Option<(Pixels, Pixels)> {
    let origin = editor.text_bounds()?.origin.y;
    let map = editor.display_map();
    let row_at = |line| {
        if line >= map.buffer_line_count() {
            map.display_row_count()
        } else {
            map.buffer_line_to_display_row(line)
        }
    };
    let line_height = editor
        .cursor_layout()
        .map(|(_, height)| height)
        .unwrap_or(px(ROW_HEIGHT));
    let top = origin + line_height * row_at(lines.start) as f32;
    let bottom = origin + line_height * row_at(lines.end) as f32;
    Some((top, bottom.max(top + line_height)))
}

/// Clip a band, retaining a visible control at the top of a long, partially scrolled hunk.
pub fn clip_band(top: f32, bottom: f32, height: f32) -> Option<(f32, f32)> {
    let start = top.max(0.);
    let end = bottom.min(height);
    (end > start).then_some((start, end - start))
}

pub fn backgrounds(
    editor: Entity<EditorState>,
    ranges: Vec<Range<usize>>,
    color: Hsla,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            let state = editor.read(cx);
            for range in &ranges {
                if range.is_empty() {
                    continue;
                }
                if let Some((top, bottom)) = vertical_range(state, range) {
                    let top = top.max(bounds.top());
                    let bottom = bottom.min(bounds.bottom());
                    if bottom > top {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.left(), top),
                                size(bounds.size.width, bottom - top),
                            ),
                            color,
                        ));
                    }
                }
            }
        },
    )
    .absolute()
    .size_full()
}

/// Editor scroll writes are deferred to prepaint. Synchronize once per frame and acknowledge
/// the target's next offset (including native clamping) without echoing it to the source.
#[derive(Default)]
pub struct ScrollSync {
    previous: [Point<Pixels>; 2],
    pending: Option<usize>,
}
impl ScrollSync {
    pub fn update(&mut self, current: [Point<Pixels>; 2]) -> Option<(usize, Point<Pixels>)> {
        let mut changed = [
            current[0] != self.previous[0],
            current[1] != self.previous[1],
        ];
        if let Some(target) = self.pending.take() {
            changed[target] = false;
        }
        self.previous = current;
        let source = if changed[1] {
            1
        } else if changed[0] {
            0
        } else {
            return None;
        };
        let target = 1 - source;
        if current[target] == current[source] {
            return None;
        }
        self.pending = Some(target);
        Some((target, current[source]))
    }
}

#[cfg(test)]
mod tests {
    use super::{ScrollSync, clip_band};
    use gpui_kit::{point, px};
    #[test]
    fn clipping_keeps_scrolled_hunks_reachable_without_offscreen_controls() {
        assert_eq!(clip_band(-300., 160., 600.), Some((0., 160.)));
        assert_eq!(clip_band(560., 900., 600.), Some((560., 40.)));
        assert_eq!(clip_band(-200., -20., 600.), None);
        assert_eq!(clip_band(700., 800., 600.), None);
    }
    #[test]
    fn linked_scrolling_does_not_echo_native_clamping_back_to_the_source() {
        let at = |y| point(px(0.), px(y));
        let mut sync = ScrollSync::default();
        assert_eq!(sync.update([at(0.), at(-3000.)]), Some((0, at(-3000.))));
        assert_eq!(sync.update([at(-1000.), at(-3000.)]), None);
        assert_eq!(sync.update([at(-1000.), at(-3000.)]), None);
        assert_eq!(sync.update([at(-800.), at(-3000.)]), Some((1, at(-800.))));
        assert_eq!(sync.update([at(-800.), at(-800.)]), None);
    }
}
