//! Shared hunk navigation and a compact overview, independent of text rendering.
use difly::model::Hunk;
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind};
use std::ops::Range;

pub const WIDTH: f32 = 18.0;
pub const GAP: f32 = 5.0;

pub fn areas(available: Rect) -> (Rect, Rect) {
    let left = (available.right() - WIDTH).max(available.left());
    let rail = Rect::from_min_max(Pos2::new(left, available.top()), available.max);
    let content = Rect::from_min_max(
        available.min,
        Pos2::new((left - GAP).max(available.left()), available.bottom()),
    );
    (content, rail)
}

pub fn next(current: Option<usize>, count: usize, backwards: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current {
        None => {
            if backwards {
                count - 1
            } else {
                0
            }
        }
        Some(index) => {
            let index = index.min(count - 1);
            if backwards {
                index.saturating_sub(1)
            } else {
                (index + 1).min(count - 1)
            }
        }
    })
}

/// Consume Shift+F7 first: egui's unmodified shortcut matcher permits extra Shift.
pub fn shortcut(ctx: &egui::Context) -> Option<bool> {
    ctx.input_mut(|input| {
        if input.consume_key(egui::Modifiers::SHIFT, egui::Key::F7) {
            Some(true)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::F7) {
            Some(false)
        } else {
            None
        }
    })
}

fn marker(track: Rect, range: &Range<f32>, height: f32) -> Rect {
    let top = track.top() + track.height() * (range.start / height).clamp(0.0, 1.0);
    let bottom = track.top() + track.height() * (range.end / height).clamp(0.0, 1.0);
    let h = (bottom - top).max(3.0).min(track.height());
    let y = top.min(track.bottom() - h);
    Rect::from_min_max(
        Pos2::new(track.left() + 3.0, y),
        Pos2::new(track.right() - 3.0, y + h),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn overview(
    ui: &mut egui::Ui,
    id: Id,
    rect: Rect,
    hunks: &[Hunk],
    ranges: &[Range<usize>],
    offsets: &[f32],
    height: f32,
    viewport: Range<f32>,
) -> Option<usize> {
    if rect.width() < 8.0 || rect.height() < 8.0 {
        return None;
    }
    let height = height.max(1.0);
    let response = ui
        .interact(rect, id, Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Change map · click to jump to a hunk\nF7: next · Shift+F7: previous");
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let track = rect.shrink2(egui::vec2(0.0, 2.0));
    let view = marker(track, &viewport, height);
    let view = Rect::from_min_max(
        Pos2::new(track.left(), view.top()),
        Pos2::new(track.right(), view.bottom()),
    );
    painter.rect_filled(
        view,
        2.0,
        ui.visuals().selection.bg_fill.gamma_multiply(0.3),
    );
    let mut nearest = None;
    let mut distance = f32::INFINITY;
    for (index, (range, hunk)) in ranges.iter().zip(hunks).enumerate() {
        let (Some(&start), Some(&end)) = (offsets.get(range.start), offsets.get(range.end)) else {
            continue;
        };
        let mark = marker(track, &(start + 28.0..end + 28.0), height);
        let color = if hunk.old.is_empty() {
            Color32::from_rgb(39, 157, 81)
        } else if hunk.new.is_empty() {
            Color32::from_rgb(212, 74, 74)
        } else {
            Color32::from_rgb(68, 135, 218)
        };
        painter.rect_filled(mark, 1.0, color);
        if let Some(pos) = response.interact_pointer_pos() {
            let d = (mark.center().y - pos.y).abs();
            if d < distance {
                distance = d;
                nearest = Some(index);
            }
        }
    }
    painter.rect_stroke(
        view,
        2.0,
        Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.fg_stroke.color),
        StrokeKind::Inside,
    );
    if response.clicked() || response.dragged() {
        nearest
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_includes_first_hunk_and_handles_edges_and_changed_counts() {
        assert_eq!(next(None, 3, false), Some(0));
        assert_eq!(next(Some(0), 3, false), Some(1));
        assert_eq!(next(Some(1), 3, true), Some(0));
        assert_eq!(next(Some(0), 3, true), Some(0));
        assert_eq!(next(Some(2), 3, false), Some(2));
        assert_eq!(next(None, 3, true), Some(2));
        assert_eq!(next(Some(9), 2, false), Some(1));
        assert_eq!(next(Some(0), 0, false), None);
    }
    #[test]
    fn f7_shortcuts_are_consumed_once_with_correct_direction() {
        let ctx = egui::Context::default();
        for (modifiers, expected) in [
            (egui::Modifiers::NONE, Some(false)),
            (egui::Modifiers::SHIFT, Some(true)),
            (egui::Modifiers::COMMAND, None),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::F7,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    assert_eq!(shortcut(ctx), expected);
                    assert_eq!(shortcut(ctx), None);
                },
            );
        }
    }
    #[test]
    fn map_markers_stay_visible_at_document_edges() {
        let track = Rect::from_min_size(Pos2::ZERO, egui::vec2(18.0, 100.0));
        let top = marker(track, &(0.0..1.0), 10000.0);
        let bottom = marker(track, &(9999.0..10000.0), 10000.0);
        assert_eq!(top.top(), 0.0);
        assert_eq!(bottom.bottom(), 100.0);
        assert!(top.height() >= 3.0 && bottom.height() >= 3.0);
    }
    #[test]
    fn overview_click_selects_hunk_without_changing_inclusion() {
        let ctx = egui::Context::default();
        let hunks = difly::model::hunks("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\n");
        let mut clicked = None;
        for pressed in [None, Some(true), Some(false)] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(300.0, 300.0))),
                    events: pressed.map_or_else(Vec::new, |pressed| {
                        vec![
                            egui::Event::PointerMoved(Pos2::new(210.0, 195.0)),
                            egui::Event::PointerButton {
                                pos: Pos2::new(210.0, 195.0),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    }),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        clicked = overview(
                            ui,
                            Id::new("map"),
                            Rect::from_min_max(Pos2::new(200.0, 0.0), Pos2::new(218.0, 240.0)),
                            &hunks,
                            &[0..1, 4..5],
                            &[0.0, 200.0, 400.0, 600.0, 800.0, 1000.0],
                            1028.0,
                            0.0..300.0,
                        )
                        .or(clicked);
                    });
                },
            );
        }
        assert_eq!(clicked, Some(1));
        assert!(hunks.iter().all(|h| h.selected));
    }
}
