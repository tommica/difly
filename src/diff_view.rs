//! Shared geometry for editors, line numbers, hunk controls, and backgrounds.
//! Wrapping changes visual rows only; the editable strings remain untouched.
use difly::model::{self, DiffRows};
use eframe::egui::{self, Galley, Rect, Vec2};
use std::sync::Arc;

pub const LINE_HEIGHT: f32 = 23.0;
pub const GUTTER: f32 = 65.0;
pub const TEXT_INSET: f32 = 48.0;
pub const TEXT_PADDING: f32 = 8.0;

pub struct DiffLayout {
    pub rows: DiffRows,
    pub offsets: Vec<f32>,
    pub old: Arc<Galley>,
    pub new: Arc<Galley>,
    pub old_width: f32,
    pub new_width: f32,
    pub height: f32,
    pub line_height: f32,
}

pub fn snap(value: f32, pixels_per_point: f32) -> f32 {
    (value * pixels_per_point).round() / pixels_per_point
}

impl DiffLayout {
    pub fn build(
        ui: &egui::Ui,
        mut old_job: egui::text::LayoutJob,
        mut new_job: egui::text::LayoutJob,
        viewport_width: f32,
        unified: bool,
        wrap: bool,
        split: f32,
    ) -> Self {
        let available = (viewport_width - GUTTER).max(80.0);
        let [old_visible, new_visible] = if unified {
            [available, available]
        } else {
            split_widths(available, split)
        };
        for (job, width) in [(&mut old_job, old_visible), (&mut new_job, new_visible)] {
            job.wrap.max_width = if wrap {
                (width - TEXT_INSET - TEXT_PADDING).max(24.0)
            } else {
                f32::INFINITY
            };
            job.wrap.break_anywhere = false;
        }
        let hunks = model::hunks(&old_job.text, &new_job.text);
        let rows = model::diff_rows(&old_job.text, &new_job.text, &hunks, unified);
        let old_raw = ui.fonts_mut(|f| f.layout_job(old_job));
        let new_raw = ui.fonts_mut(|f| f.layout_job(new_job));
        let old_counts = line_counts(&old_raw);
        let new_counts = line_counts(&new_raw);
        let ppp = ui.ctx().pixels_per_point();
        let max_row = old_raw
            .rows
            .iter()
            .chain(&new_raw.rows)
            .map(|r| r.size.y)
            .fold(LINE_HEIGHT, f32::max);
        let line_height = (max_row * ppp).ceil() / ppp;
        let mut offsets = vec![0.0];
        for (old, new) in &rows.rows {
            let count = old
                .map(|i| old_counts[i])
                .unwrap_or(0)
                .max(new.map(|i| new_counts[i]).unwrap_or(0))
                .max(1);
            offsets.push(offsets.last().unwrap() + count as f32 * line_height);
        }
        let height = offsets.last().unwrap() + 2.0 * line_height;
        let (old_width, new_width) = if wrap {
            (old_visible, new_visible)
        } else if unified {
            let width = available
                .max(old_raw.rect.width().max(new_raw.rect.width()) + TEXT_INSET + TEXT_PADDING);
            (width, width)
        } else {
            let ratio = old_visible / available;
            let total = available
                .max((old_raw.rect.width() + TEXT_INSET + TEXT_PADDING) / ratio)
                .max((new_raw.rect.width() + TEXT_INSET + TEXT_PADDING) / (1.0 - ratio));
            (total * ratio, total * (1.0 - ratio))
        };
        let old = position_rows(&old_raw, &rows.old_y, &offsets, line_height, height, ppp);
        let new = position_rows(&new_raw, &rows.new_y, &offsets, line_height, height, ppp);
        Self {
            rows,
            offsets,
            old,
            new,
            old_width,
            new_width,
            height,
            line_height,
        }
    }

    pub fn width(&self, unified: bool) -> f32 {
        GUTTER
            + if unified {
                self.new_width
            } else {
                self.old_width + self.new_width
            }
    }
}

/// Keep both editors usable even at the limits of a divider drag.
pub fn split_widths(available: f32, ratio: f32) -> [f32; 2] {
    let minimum = 140.0_f32.min(available * 0.5);
    let left = (available * ratio).clamp(minimum, available - minimum);
    [left, available - left]
}

fn line_counts(galley: &Galley) -> Vec<usize> {
    let mut counts = Vec::new();
    let mut count = 0;
    for row in &galley.rows {
        count += 1;
        if row.ends_with_newline {
            counts.push(count);
            count = 0;
        }
    }
    if count > 0 {
        counts.push(count);
    }
    counts
}

fn position_rows(
    raw: &Galley,
    map: &[usize],
    offsets: &[f32],
    line_height: f32,
    height: f32,
    ppp: f32,
) -> Arc<Galley> {
    let mut galley = raw.clone();
    let (mut logical, mut continuation) = (0, 0);
    let mut bounds = Rect::NOTHING;
    for row in &mut galley.rows {
        let display_row = map.get(logical).copied().unwrap_or(offsets.len() - 1);
        let top = offsets[display_row];
        row.pos.y = snap(
            top + continuation as f32 * line_height + (line_height - row.size.y) * 0.5,
            ppp,
        );
        row.pos.x = snap(row.pos.x, ppp);
        bounds = bounds.union(row.visuals.mesh_bounds.translate(row.pos.to_vec2()));
        continuation += 1;
        if row.ends_with_newline {
            logical += 1;
            continuation = 0;
        }
    }
    galley.rect.max.y = height;
    galley.mesh_bounds = bounds;
    Arc::new(galley)
}

pub fn line_rect(origin: egui::Pos2, width: f32, offsets: &[f32], row: usize) -> Rect {
    Rect::from_min_size(
        origin + Vec2::new(0.0, offsets[row]),
        Vec2::new(width, offsets[row + 1] - offsets[row]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(text: &str) -> egui::text::LayoutJob {
        egui::text::LayoutJob::simple(
            text.into(),
            egui::FontId::monospace(15.0),
            egui::Color32::BLACK,
            f32::INFINITY,
        )
    }
    #[test]
    fn unequal_panes_rewrap_independently_and_keep_matching_lines_aligned() {
        let ctx = egui::Context::default();
        let old = format!("{}\nunchanged\n", "old content ".repeat(20));
        let new = format!("{}\nunchanged\n", "new content ".repeat(30));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut previous_width = 0.0;
                for ratio in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let view =
                        DiffLayout::build(ui, job(&old), job(&new), 900.0, false, true, ratio);
                    assert!(view.old_width >= 140.0 && view.new_width >= 140.0);
                    assert!(view.old_width >= previous_width);
                    previous_width = view.old_width;
                    assert!((view.width(false) - 900.0).abs() < 0.01);
                    let old_equal = view
                        .old
                        .rows
                        .iter()
                        .find(|r| r.glyphs.first().is_some_and(|g| g.chr == 'u'))
                        .unwrap();
                    let new_equal = view
                        .new
                        .rows
                        .iter()
                        .find(|r| r.glyphs.first().is_some_and(|g| g.chr == 'u'))
                        .unwrap();
                    assert_eq!(old_equal.pos.y, new_equal.pos.y);
                    assert_eq!(view.old.job.text, old);
                    assert_eq!(view.new.job.text, new);
                }
            });
        });
    }

    #[test]
    fn wrapped_lines_share_height_and_do_not_change_source_text() {
        let ctx = egui::Context::default();
        let old = "short\nunchanged\n";
        let new =
            "a much longer replacement line that must wrap across several visual rows\nunchanged\n";
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for unified in [false, true] {
                    let view = DiffLayout::build(ui, job(old), job(new), 300.0, unified, true, 0.5);
                    assert_eq!(view.new.job.text, new);
                    assert!(view.new.rows.len() > new.lines().count() + 1);
                    let old_equal = view
                        .old
                        .rows
                        .iter()
                        .find(|r| r.glyphs.first().is_some_and(|g| g.chr == 'u'))
                        .unwrap();
                    let new_equal = view
                        .new
                        .rows
                        .iter()
                        .find(|r| r.glyphs.first().is_some_and(|g| g.chr == 'u'))
                        .unwrap();
                    assert_eq!(old_equal.pos.y, new_equal.pos.y);
                    assert!(view.width(unified) <= 300.1);
                }
            });
        });
    }
    #[test]
    fn unwrapped_highlights_cover_long_text_and_scrolled_continuations() {
        let ctx = egui::Context::default();
        let new = format!("{}\n", "long_code ".repeat(100));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let view = DiffLayout::build(ui, job(""), job(&new), 500.0, true, false, 0.5);
                assert!(view.new_width > 1000.0);
                assert!(view.new_width >= view.new.rect.width() + TEXT_INSET);
                let rect = line_rect(egui::pos2(-1000.0, 0.0), view.new_width, &view.offsets, 0);
                assert!(rect.contains(egui::pos2(10.0, 5.0)));
                let wrapped = DiffLayout::build(ui, job(""), job(&new), 300.0, true, true, 0.5);
                let rect = line_rect(
                    egui::pos2(0.0, -LINE_HEIGHT * 3.0),
                    wrapped.new_width,
                    &wrapped.offsets,
                    0,
                );
                assert!(rect.contains(egui::pos2(10.0, 5.0)));
            });
        });
    }
    #[test]
    fn custom_row_positions_are_on_the_physical_pixel_grid() {
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let view = DiffLayout::build(
                        ui,
                        job("a\nb\n"),
                        job("inserted\na\nlong line that wraps onto more rows\nb\n"),
                        300.0,
                        false,
                        true,
                        0.5,
                    );
                    for row in view.old.rows.iter().chain(&view.new.rows) {
                        let y = row.pos.y * ctx.pixels_per_point();
                        assert!((y - y.round()).abs() < 0.001);
                    }
                });
            });
        }
    }
}
