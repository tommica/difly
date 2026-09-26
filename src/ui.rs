use crate::diff_view::{self, DiffLayout, GUTTER, TEXT_INSET, TEXT_PADDING};
use crate::file_tree::FileTree;
use difly::{
    git::{CommitInfo, File, Repo},
    model::{self},
};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

const CODE_SIZE: f32 = 15.0;
#[derive(Clone, Copy)]
pub(crate) struct Colors {
    pub(crate) bg: Color32,
    pub(crate) panel: Color32,
    pub(crate) muted: Color32,
    pub(crate) accent: Color32,
    pub(crate) green: Color32,
    pub(crate) add: Color32,
    pub(crate) del: Color32,
    pub(crate) border: Color32,
    pub(crate) warn: Color32,
    pub(crate) error_bg: Color32,
    pub(crate) warning_bg: Color32,
    pub(crate) included: Color32,
}
impl Colors {
    pub(crate) fn new(dark: bool) -> Self {
        let rgb = |(r, g, b)| Color32::from_rgb(r, g, b);
        let values = if dark {
            [
                (20, 23, 30),
                (26, 30, 39),
                (139, 151, 170),
                (139, 162, 255),
                (139, 210, 169),
                (29, 55, 45),
                (62, 36, 44),
                (65, 72, 85),
                (235, 191, 94),
                (67, 35, 42),
                (50, 43, 31),
            ]
        } else {
            [
                (255, 255, 255),
                (237, 240, 245),
                (65, 76, 94),
                (25, 65, 155),
                (40, 117, 68),
                (209, 233, 210),
                (248, 213, 216),
                (150, 161, 180),
                (146, 99, 18),
                (255, 231, 234),
                (255, 246, 223),
            ]
        };
        Self {
            bg: rgb(values[0]),
            panel: rgb(values[1]),
            muted: rgb(values[2]),
            accent: rgb(values[3]),
            green: rgb(values[4]),
            add: rgb(values[5]),
            del: rgb(values[6]),
            border: rgb(values[7]),
            warn: rgb(values[8]),
            error_bg: rgb(values[9]),
            warning_bg: rgb(values[10]),
            included: if dark {
                rgb((63, 57, 102))
            } else {
                rgb((198, 184, 249))
            },
        }
    }
}
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrains Mono".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"))
            .into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrains Mono".into());
    ctx.set_fonts(fonts);
}
fn apply_theme(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
    let colors = Colors::new(dark);
    let mut style = (*ctx.style()).clone();
    style.visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.panel_fill = colors.panel;
    style.visuals.window_fill = colors.panel;
    style.visuals.extreme_bg_color = colors.bg;
    style.visuals.selection.bg_fill = if dark {
        Color32::from_rgb(56, 72, 117)
    } else {
        Color32::from_rgb(210, 223, 252)
    };
    if !dark {
        let text = Color32::from_rgb(24, 31, 43);
        style.visuals.override_text_color = Some(text);
        style.visuals.weak_text_color = Some(colors.muted);
        style.visuals.hyperlink_color = colors.accent;
        style.visuals.warn_fg_color = Color32::from_rgb(125, 77, 8);
        style.visuals.error_fg_color = Color32::from_rgb(155, 34, 48);
        style.visuals.selection.bg_fill = Color32::from_rgb(190, 211, 249);
        style.visuals.selection.stroke = Stroke::new(1.0_f32, text);
        for widget in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            widget.fg_stroke = Stroke::new(1.0_f32, text);
            widget.bg_stroke = Stroke::new(1.0_f32, colors.border);
        }
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(222, 227, 236);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(202, 215, 239);
        style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(181, 201, 238);
    }
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(CODE_SIZE));
    ctx.set_style(style);
}

enum Task {
    Scan(u64, anyhow::Result<Vec<File>>),
    Commit(anyhow::Result<String>),
}
struct Undo {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    before_mode: String,
    after_mode: String,
}
pub(crate) struct Highlight {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
    cache: std::collections::VecDeque<(String, std::sync::Arc<Highlighted>)>,
    budget: usize,
}
#[derive(Clone)]
struct Span {
    range: std::ops::Range<usize>,
    color: Color32,
}
pub(crate) struct Highlighted {
    text: String,
    extension: String,
    dark: bool,
    spans: Vec<Span>,
}
impl Highlighted {
    fn bytes(&self) -> usize {
        self.text.capacity()
            + self.extension.capacity()
            + self.spans.capacity() * std::mem::size_of::<Span>()
    }
    pub(crate) fn job(&self, range: std::ops::Range<usize>) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        for span in self
            .spans
            .iter()
            .skip(self.spans.partition_point(|s| s.range.end <= range.start))
        {
            if span.range.start >= range.end {
                break;
            }
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            job.append(
                &self.text[start..end],
                0.0,
                egui::TextFormat {
                    font_id: FontId::monospace(CODE_SIZE),
                    color: span.color,
                    ..Default::default()
                },
            );
        }
        job
    }
}
impl Highlight {
    pub(crate) fn new() -> Self {
        Self {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            themes: ThemeSet::load_defaults(),
            cache: Default::default(),
            budget: 8 * 1024 * 1024,
        }
    }
    pub(crate) fn document(
        &mut self,
        slot: &str,
        text: &str,
        extension: &str,
        dark: bool,
    ) -> std::sync::Arc<Highlighted> {
        if let Some(i) = self.cache.iter().position(|(key, doc)| {
            key == slot && doc.text == text && doc.extension == extension && doc.dark == dark
        }) {
            let entry = self.cache.remove(i).unwrap();
            let doc = entry.1.clone();
            self.cache.push_back(entry);
            return doc;
        }
        self.cache.retain(|(key, _)| key != slot);
        let syntax = self
            .syntaxes
            .find_syntax_by_extension(extension)
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text());
        let mut highlighter = HighlightLines::new(
            syntax,
            &self.themes.themes[if dark {
                "base16-ocean.dark"
            } else {
                "InspiredGitHub"
            }],
        );
        let mut spans: Vec<Span> = vec![];
        let mut palette = HashMap::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let parts = highlighter
                .highlight_line(line, &self.syntaxes)
                .map(|parts| {
                    parts
                        .into_iter()
                        .map(|(style, part)| {
                            let color = *palette
                                .entry(style.foreground)
                                .or_insert_with(|| syntax_color(style.foreground, dark));
                            (color, part)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|_| {
                    vec![(
                        if dark {
                            Color32::LIGHT_GRAY
                        } else {
                            Color32::from_gray(40)
                        },
                        line,
                    )]
                });
            for (color, part) in parts {
                if let Some(last) = spans.last_mut()
                    && last.color == color
                {
                    last.range.end += part.len();
                } else {
                    spans.push(Span {
                        range: offset..offset + part.len(),
                        color,
                    });
                }
                offset += part.len();
            }
        }
        let doc = std::sync::Arc::new(Highlighted {
            text: text.into(),
            extension: extension.into(),
            dark,
            spans,
        });
        let size = doc.bytes() + slot.len();
        while self
            .cache
            .iter()
            .map(|(key, d)| key.capacity() + d.bytes())
            .sum::<usize>()
            + size
            > self.budget
            && !self.cache.is_empty()
        {
            self.cache.pop_front();
        }
        if size <= self.budget {
            self.cache.push_back((slot.into(), doc.clone()));
        }
        doc
    }
    fn job_for(
        &mut self,
        slot: &str,
        text: &str,
        extension: &str,
        dark: bool,
    ) -> egui::text::LayoutJob {
        self.document(slot, text, extension, dark)
            .job(0..text.len())
    }
    #[cfg(test)]
    fn job(&mut self, text: &str, extension: &str, dark: bool) -> egui::text::LayoutJob {
        self.job_for("test", text, extension, dark)
    }
}

pub struct App {
    repo: Option<Repo>,
    files: Vec<File>,
    active: Option<PathBuf>,
    draft: String,
    message: String,
    amend: bool,
    amend_target: Option<String>,
    pre_amend_message: Option<String>,
    recent_commits: Vec<CommitInfo>,
    unified: bool,
    soft_wrap: bool,
    split_ratio: f32,
    head: Option<String>,
    branch: String,
    merge: bool,
    error: Option<String>,
    notice: String,
    busy: bool,
    scanning: bool,
    tx: mpsc::Sender<Task>,
    rx: mpsc::Receiver<Task>,
    epoch: u64,
    last_scan: Instant,
    last_edit: Instant,
    highlight: Highlight,
    virtual_diff: crate::virtual_diff::VirtualDiff,
    open: bool,
    open_path: String,
    filter: String,
    file_tree: FileTree,
    confirm_revert: bool,
    undo: Vec<Undo>,
    conflict_sources: [String; 3],
    source_tab: usize,
    saved: bool,
    jump_hunk: Option<usize>,
    hunk_cursor: Option<usize>,
    confirm_close: bool,
    dark: bool,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, path: PathBuf) -> Self {
        install_fonts(&cc.egui_ctx);
        let dark = load_dark_theme(cc.storage);
        apply_theme(&cc.egui_ctx, dark);
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            repo: None,
            files: Vec::new(),
            active: None,
            draft: String::new(),
            message: String::new(),
            amend: false,
            amend_target: None,
            pre_amend_message: None,
            recent_commits: vec![],
            unified: false,
            soft_wrap: true,
            split_ratio: 0.5,
            head: None,
            branch: String::new(),
            merge: false,
            error: None,
            notice: "Ready".into(),
            busy: false,
            scanning: false,
            tx,
            rx,
            epoch: 0,
            last_scan: Instant::now(),
            last_edit: Instant::now(),
            highlight: Highlight::new(),
            virtual_diff: Default::default(),
            open: false,
            open_path: path.display().to_string(),
            filter: String::new(),
            file_tree: FileTree::default(),
            confirm_revert: false,
            undo: Vec::new(),
            conflict_sources: Default::default(),
            source_tab: 1,
            saved: true,
            jump_hunk: None,
            hunk_cursor: None,
            confirm_close: false,
            dark,
        };
        match Repo::open(&path) {
            Ok(repo) => {
                app.head = repo.head();
                app.recent_commits = repo.recent_commits().unwrap_or_default();
                app.branch = repo.branch();
                app.merge = repo.merge();
                if app.merge {
                    app.message = repo.merge_message();
                }
                app.repo = Some(repo);
                app.scan();
            }
            Err(e) => {
                app.error = Some(e.to_string());
                app.open = true;
            }
        }
        app
    }
    fn scan(&mut self) {
        if self.scanning || self.busy || !self.saved {
            return;
        }
        if let Some(repo) = self.repo.clone() {
            let tx = self.tx.clone();
            let epoch = self.epoch;
            self.scanning = true;
            self.last_scan = Instant::now();
            let active = self.active.clone();
            std::thread::spawn(move || {
                let result = repo.scan_metadata().and_then(|mut files| {
                    if let Some(file) = files.iter_mut().find(|f| Some(&f.path) == active.as_ref())
                    {
                        repo.load_preview(file)?;
                    }
                    Ok(files)
                });
                let _ = tx.send(Task::Scan(epoch, result));
            });
        }
    }
    fn apply_scan(&mut self, mut files: Vec<File>) {
        for f in &mut files {
            if let Some(prev) = self
                .files
                .iter()
                .find(|p| p.path == f.path && p.conflict == f.conflict)
            {
                if f.source == prev.source {
                    if f.loaded {
                        for h in &mut f.hunks {
                            if let Some(previous) =
                                prev.hunks.iter().find(|p| p.old == h.old && p.new == h.new)
                            {
                                h.selected = previous.selected;
                                h.review = previous.review;
                            } else {
                                h.selected = prev.selected;
                            }
                        }
                    } else {
                        f.hunks = prev.hunks.clone();
                        for h in &mut f.hunks {
                            h.added = String::new();
                            h.removed = String::new();
                        }
                    }
                    f.selected = prev.selected;
                    f.binary = prev.binary;
                } else if f.loaded && prev.loaded && f.old == prev.old {
                    f.hunks =
                        model::refresh_hunks(&f.old_text(), &f.text(), &prev.hunks, prev.selected);
                    f.selected = prev.selected && f.hunks.iter().all(|h| h.selected);
                } else {
                    // Unopened whole-file choices survive refresh. Partial selections need review.
                    f.selected = prev.selected;
                    if !prev.hunks.is_empty()
                        && prev
                            .hunks
                            .iter()
                            .any(|h| h.selected != prev.selected || h.review)
                    {
                        f.hunks = prev.hunks.clone();
                        for h in &mut f.hunks {
                            h.review = true;
                            h.selected = false;
                            h.added.clear();
                            h.removed.clear();
                        }
                        f.selected = false;
                    }
                }
            }
        }
        let active = self.active.clone();
        self.files = files;
        let next = active
            .filter(|p| self.files.iter().any(|f| &f.path == p))
            .or_else(|| self.files.first().map(|f| f.path.clone()));
        if next != self.active {
            if let Some(path) = next {
                self.activate(path);
            } else {
                self.active = None;
                self.draft.clear();
            }
        } else if let Some(i) = self.index() {
            self.draft = if self.files[i].binary {
                String::new()
            } else {
                self.files[i].text()
            };
        }
    }
    fn index(&self) -> Option<usize> {
        self.active
            .as_ref()
            .and_then(|p| self.files.iter().position(|f| &f.path == p))
    }
    fn activate(&mut self, path: PathBuf) {
        if !self.saved {
            self.error = Some("Save or reload the current edit before switching files.".into());
            return;
        }
        self.epoch += 1; // Discard an in-flight scan of the previous active file.
        self.virtual_diff = Default::default();
        self.conflict_sources = Default::default();
        for f in &mut self.files {
            if f.path != path {
                f.unload();
            }
        }
        self.active = Some(path.clone());
        self.hunk_cursor = None;
        self.jump_hunk = None;
        if let Some(i) = self.index() {
            if let Some(repo) = &self.repo
                && let Err(error) = repo.load_preview(&mut self.files[i])
            {
                self.error = Some(error.to_string());
                self.draft.clear();
                return;
            }
            self.draft = if self.files[i].binary {
                String::new()
            } else {
                self.files[i].text()
            };
            if self.files[i].conflict
                && let Some(r) = &self.repo
            {
                for stage in 1..=3 {
                    self.conflict_sources[stage - 1] =
                        r.conflict_stage(&path, stage as u8).unwrap_or_default();
                }
            }
        }
    }
    fn save(&mut self) {
        let Some(i) = self.index() else {
            return;
        };
        let Some(r) = &self.repo else {
            return;
        };
        self.epoch += 1;
        self.last_edit = Instant::now();
        match r.save(&mut self.files[i], &self.draft) {
            Ok(()) => {
                self.saved = true;
                self.notice = "All edits saved".into();
            }
            Err(e) => {
                self.saved = false;
                self.error = Some(format!("Autosave stopped: {e}"));
            }
        }
    }
    fn commit(&mut self) {
        if self.busy || !self.saved {
            return;
        }
        if let Some(repo) = self.repo.clone() {
            self.busy = true;
            self.epoch += 1;
            self.notice = "Committing…".into();
            let (files, message, amend, target, head, tx) = (
                self.files.clone(),
                self.message.clone(),
                self.amend,
                self.amend_target.clone(),
                self.head.clone(),
                self.tx.clone(),
            );
            std::thread::spawn(move || {
                let result = if amend && target.is_some() {
                    repo.amend_commit(
                        &files,
                        &message,
                        target.as_deref().unwrap(),
                        head.as_deref(),
                    )
                } else {
                    repo.commit(&files, &message, amend, head.as_deref())
                };
                let _ = tx.send(Task::Commit(result));
            });
        }
    }
    fn revert(&mut self, hunk: Option<usize>) {
        let Some(i) = self.index() else {
            return;
        };
        let Some(repo) = &self.repo else {
            return;
        };
        if let Err(error) = repo.load_file(&mut self.files[i]) {
            self.error = Some(error.to_string());
            return;
        }
        let f = &self.files[i];
        let before = f.exists.then(|| f.current.clone());
        let after = if let Some(h) = hunk {
            Some(model::revert_hunk(&self.draft, &f.hunks[h]).into_bytes())
        } else if f.base_exists {
            Some(f.old.clone())
        } else {
            None
        };
        let after_mode = if hunk.is_some() {
            f.mode.clone()
        } else {
            f.old_mode.clone()
        };
        match repo.restore_work(f, after.as_deref(), &after_mode) {
            Ok(()) => {
                self.undo.push(Undo {
                    path: f.path.clone(),
                    before,
                    after: after.clone(),
                    before_mode: f.mode.clone(),
                    after_mode: after_mode.clone(),
                });
                self.epoch += 1;
                self.draft = String::from_utf8_lossy(after.as_deref().unwrap_or_default()).into();
                self.files[i].current = after.clone().unwrap_or_default();
                self.files[i].exists = after.is_some();
                self.files[i].mode = after_mode;
                self.files[i].hunks = model::refresh_hunks(
                    &self.files[i].old_text(),
                    &self.draft,
                    &self.files[i].hunks,
                    true,
                );
                self.notice = "Reverted · Undo revert is available".into();
                self.scan();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn undo_revert(&mut self) {
        let Some(u) = self.undo.last() else {
            return;
        };
        let Some(r) = &self.repo else {
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                r.read_work(&u.path)? == u.after,
                "File changed since revert; undo would overwrite newer edits"
            );
            let file = File {
                path: u.path.clone(),
                old: Vec::new(),
                current: u.after.clone().unwrap_or_default(),
                exists: u.after.is_some(),
                base_exists: true,
                tracked: true,
                conflict: false,
                binary: false,
                mode: u.after_mode.clone(),
                old_mode: u.before_mode.clone(),
                hunks: vec![],
                selected: true,
                loaded: true,
                source: None,
            };
            r.restore_work(&file, u.before.as_deref(), &u.before_mode)
        })();
        match result {
            Ok(()) => {
                self.undo.pop();
                self.epoch += 1;
                self.scan();
                self.notice = "Revert undone".into();
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn poll(&mut self) {
        while let Ok(task) = self.rx.try_recv() {
            match task {
                Task::Scan(epoch, result) => {
                    self.scanning = false;
                    if epoch == self.epoch {
                        match result {
                            Ok(files) => self.apply_scan(files),
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                }
                Task::Commit(result) => {
                    self.busy = false;
                    match result {
                        Ok(msg) => {
                            self.notice = msg.lines().next().unwrap_or("Committed").into();
                            self.message.clear();
                            self.amend = false;
                            self.amend_target = None;
                            self.pre_amend_message = None;
                            self.files.clear();
                            self.active = None;
                            self.undo.clear();
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                    if let Some(r) = &self.repo {
                        self.head = r.head();
                        self.recent_commits = r.recent_commits().unwrap_or_default();
                        self.merge = r.merge();
                        self.branch = r.branch();
                    }
                    self.scan();
                }
            }
        }
        if self.last_scan.elapsed() > Duration::from_secs(3)
            && self.last_edit.elapsed() > Duration::from_millis(800)
        {
            self.scan();
        }
    }
    fn header(&mut self, ctx: &egui::Context) {
        let colors = Colors::new(self.dark);
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::new().fill(colors.panel).inner_margin(16))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("difly")
                            .size(22.0)
                            .strong()
                            .color(colors.accent),
                    );
                    ui.separator();
                    let name = self
                        .repo
                        .as_ref()
                        .and_then(|r| r.root.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or("Open a repository".into());
                    ui.label(RichText::new(name).strong());
                    ui.label(RichText::new(format!("branch: {}", self.branch)).color(colors.muted));
                    if self.merge {
                        ui.label(
                            RichText::new("MERGE IN PROGRESS")
                                .small()
                                .color(colors.warn),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(if self.dark {
                                "Light theme"
                            } else {
                                "Dark theme"
                            })
                            .clicked()
                        {
                            self.dark = !self.dark;
                            apply_theme(ctx, self.dark);
                        }
                        if ui.button("Open repository…").clicked() {
                            self.open = true;
                        }
                        if ui
                            .add_enabled(!self.busy && !self.scanning, egui::Button::new("Refresh"))
                            .clicked()
                        {
                            if let Some(r) = &self.repo {
                                self.head = r.head();
                                self.recent_commits = r.recent_commits().unwrap_or_default();
                                self.branch = r.branch();
                                self.merge = r.merge();
                            }
                            self.scan();
                        }
                        if ui
                            .add_enabled(
                                !self.undo.is_empty() && !self.busy,
                                egui::Button::new("Undo revert"),
                            )
                            .clicked()
                        {
                            self.undo_revert();
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(colors.panel).inner_margin(10))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.busy || self.scanning {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.notice).small().color(colors.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("⌘/Ctrl Enter  commit    •    Edits autosave")
                                .small()
                                .color(colors.muted),
                        );
                    });
                });
            });
        if self.error.is_some() {
            egui::TopBottomPanel::top("error")
                .frame(egui::Frame::new().fill(colors.error_bg).inner_margin(12))
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(self.error.clone().unwrap_or_default());
                        if ui.button("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                    if !self.saved {
                        ui.label(
                            "Your unsaved text is still in the editor. Copy it before reloading.",
                        );
                        if ui
                            .button("Discard unsaved buffer and reload from disk")
                            .clicked()
                        {
                            self.saved = true;
                            self.error = None;
                            self.epoch += 1;
                            self.scan();
                        }
                    }
                });
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        let colors = Colors::new(self.dark);
        egui::SidePanel::left("files")
            .default_width(310.0)
            .min_width(240.0)
            .max_width(550.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(colors.panel).inner_margin(14))
            .show(ctx, |ui| {
                if self.busy {
                    ui.disable();
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("COMMIT").strong().color(colors.muted));
                    ui.label(format!("{} files", self.files.len()));
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Filter files…")
                        .desired_width(f32::INFINITY),
                );
                ui.checkbox(&mut self.file_tree.grouped, "Group by folder");
                let height = (ui.available_height() - 310.0).max(80.0);
                let navigation = if self.busy {
                    Default::default()
                } else {
                    self.file_tree.navigate(ctx, &mut self.files)
                };
                let mut chosen = navigation.chosen;
                if navigation.selection_changed {
                    self.epoch += 1;
                }
                self.file_tree.begin_frame();
                egui::ScrollArea::vertical()
                    .id_salt("filetree")
                    .max_height(height)
                    .min_scrolled_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let background = ui.interact(
                            ui.max_rect().intersect(ui.clip_rect()),
                            ui.id().with("tree-background"),
                            egui::Sense::click().difference(egui::Sense::focusable_noninteractive()),
                        );
                        ui.spacing_mut().indent = 28.0;
                        for (label, tracked) in [("Changes", true), ("Unversioned Files", false)] {
                            let indices: Vec<usize> = self
                                .files
                                .iter()
                                .enumerate()
                                .filter(|(_, f)| {
                                    f.tracked == tracked
                                        && f.path
                                            .to_string_lossy()
                                            .to_lowercase()
                                            .contains(&self.filter.to_lowercase())
                                })
                                .map(|(i, _)| i)
                                .collect();
                            if indices.is_empty() {
                                continue;
                            }
                            let action = self.file_tree.show_group(
                                ui,
                                &mut self.files,
                                &indices,
                                label,
                                self.active.as_ref(),
                            );
                            if action.chosen.is_some() {
                                chosen = action.chosen;
                            }
                            if action.selection_changed {
                                self.epoch += 1;
                            }
                            ui.add_space(14.0);
                        }
                        if self.files.is_empty() {
                            ui.add_space(25.0);
                            ui.label(RichText::new("Working tree is clean").color(colors.muted));
                        }
                        if background.clicked() {
                            self.file_tree.focus_area(ctx, self.active.as_ref());
                        }
                    });
                if let Some(path) = chosen {
                    self.activate(path);
                }
                ui.separator();
                let previous = (self.amend, self.amend_target.clone());
                ui.add_enabled_ui(!self.merge && self.head.is_some(), |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.amend, "Amend");
                        let title = self.amend_target.as_ref()
                            .and_then(|id| self.recent_commits.iter().find(|c| &c.oid == id))
                            .map(|c| c.subject()).unwrap_or("last commit");
                        egui::ComboBox::from_id_salt("amend-target")
                            .selected_text(title).width(180.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(self.amend_target.is_none(), "last commit").clicked() {
                                    self.amend_target = None;
                                    self.amend = true;
                                }
                                for commit in self.recent_commits.iter().skip(1) {
                                    if ui.selectable_label(self.amend_target.as_ref() == Some(&commit.oid),
                                        format!("{}  {}", &commit.oid[..8], commit.subject())).clicked() {
                                        self.amend_target = Some(commit.oid.clone());
                                        self.amend = true;
                                    }
                                }
                            });
                    });
                });
                if previous != (self.amend, self.amend_target.clone()) {
                    let target_message = self.amend_target.as_ref()
                        .and_then(|id| self.recent_commits.iter().find(|c| &c.oid == id))
                        .or_else(|| self.recent_commits.first())
                        .map(|c| c.message.as_str()).unwrap_or_default();
                    update_amend_message(&mut self.message, &mut self.pre_amend_message,
                        previous.0, self.amend, target_message);
                }
                if self.amend && let Some(target) = &self.amend_target {
                    ui.label(RichText::new(format!("Amend {} and replay newer commits", &target[..8])).small().color(colors.muted));
                    ui.label(RichText::new("Commit IDs will change. Shared branches may need a coordinated force-push.").small().color(colors.warn));
                }
                ui.add(
                    egui::TextEdit::multiline(&mut self.message)
                        .hint_text("Commit message\n\nDescribe why this change matters…")
                        .desired_width(f32::INFINITY)
                        .desired_rows(7),
                );
                let count = self.files.iter().filter(|f| f.included()).count();
                let reviews = self
                    .files
                    .iter()
                    .flat_map(|f| &f.hunks)
                    .filter(|h| h.review)
                    .count();
                ui.label(
                    RichText::new(format!("{count} files selected"))
                        .small()
                        .color(colors.muted),
                );
                if reviews > 0 {
                    ui.label(
                        RichText::new(format!("{reviews} changed hunks need review"))
                            .small()
                            .color(colors.warn),
                    );
                }
                let enabled = !self.busy
                    && self.saved
                    && !self.message.trim().is_empty()
                    && reviews == 0
                    && (count > 0 || self.amend || self.merge)
                    && !self.files.iter().any(|f| f.conflict);
                let mut label = RichText::new(if self.amend {
                    "Amend commit"
                } else if self.merge {
                    "Commit merge"
                } else {
                    "Commit"
                })
                .strong()
                .color(if self.dark {
                    Color32::WHITE
                } else {
                    colors.accent
                });
                if !enabled {
                    label = label.strikethrough();
                }
                let button = egui::Button::new(label)
                    .fill(if self.dark {
                        Color32::from_rgb(54, 76, 150)
                    } else {
                        Color32::from_rgb(220, 232, 252)
                    })
                    .stroke(Stroke::new(1.0_f32, colors.accent))
                    .corner_radius(5);
                let clicked = ui
                    .scope(|ui| {
                        // Preserve the button's colors even when the entire sidebar is disabled.
                        ui.set_opacity(1.0);
                        ui.visuals_mut().disabled_alpha = 1.0;
                        ui.add_enabled(
                            enabled,
                            button.min_size(Vec2::new(ui.available_width(), 36.0)),
                        )
                        .clicked()
                    })
                    .inner;
                if clicked {
                    self.commit();
                }
                ui.label(
                    RichText::new("Remaining changes stay unstaged.")
                        .small()
                        .color(colors.muted),
                );
            });
    }
    fn editor(&mut self, ui: &mut egui::Ui) {
        let colors = Colors::new(self.dark);
        let Some(i) = self.index() else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(140.0);
                    ui.label(
                        RichText::new("Ready for your next change")
                            .size(24.0)
                            .color(colors.muted),
                    );
                    ui.label("Select a changed file to review its diff.");
                });
            });
            return;
        };
        let file = self.files[i].clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new(file.path.display().to_string()).strong());
            if file.conflict {
                ui.colored_label(colors.warn, "Unresolved");
            } else if !file.tracked {
                ui.colored_label(colors.warn, "Unversioned");
            } else if !file.exists {
                ui.colored_label(
                    if ui.visuals().dark_mode {
                        Color32::LIGHT_RED
                    } else {
                        Color32::from_rgb(172, 58, 68)
                    },
                    "Deleted",
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.soft_wrap, "Soft wrap");
                ui.selectable_value(&mut self.unified, true, "Unified");
                ui.selectable_value(&mut self.unified, false, "Side by side");
                if ui
                    .add_enabled(true, egui::Button::new("Revert file…"))
                    .clicked()
                {
                    self.confirm_revert = true;
                }
            });
        });
        ui.separator();
        if file.binary {
            ui.add_space(30.0);
            ui.label("Binary file or symbolic link");
            if file.conflict {
                ui.label("Choose the complete version to resolve this conflict.");
                ui.horizontal(|ui| {
                    if ui.button("Resolve with ours").clicked() {
                        self.resolve_whole(i, 2);
                    }
                    if ui.button("Resolve with theirs").clicked() {
                        self.resolve_whole(i, 3);
                    }
                });
            }
            ui.label(format!(
                "{} bytes → {} bytes · Select the file to commit it as a whole.",
                file.sizes().0,
                file.sizes().1
            ));
            return;
        }
        if !file.loaded {
            ui.label("Unable to load this file. Refresh to retry.");
            return;
        }
        if file.conflict {
            self.conflict_bar(ui, i);
        }
        let old = file.old_text();
        let editor_id = egui::Id::new(("working", &file.path));
        let mut history_action = None;
        let hs = model::hunks(&old, &self.draft);
        if ui.is_enabled()
            && !self.open
            && !self.confirm_revert
            && !self.confirm_close
            && let Some(backwards) = crate::diff_navigation::shortcut(ui.ctx())
        {
            self.hunk_cursor = crate::diff_navigation::next(self.hunk_cursor, hs.len(), backwards);
            self.jump_hunk = self.hunk_cursor;
        }
        ui.horizontal(|ui| {
            let state =
                egui::text_edit::TextEditState::load(ui.ctx(), editor_id).unwrap_or_default();
            let current = (
                state.cursor.char_range().unwrap_or_default(),
                self.draft.clone(),
            );
            if ui
                .add_enabled(state.undoer().has_undo(&current), egui::Button::new("Undo"))
                .on_hover_text("Undo text edit (Cmd/Ctrl+Z)")
                .clicked()
            {
                history_action = Some(false);
            }
            if ui
                .add_enabled(state.undoer().has_redo(&current), egui::Button::new("Redo"))
                .on_hover_text("Redo text edit (Cmd/Ctrl+R)")
                .clicked()
            {
                history_action = Some(true);
            }
            ui.label(
                RichText::new(format!("{} hunks", file.hunks.len()))
                    .small()
                    .color(colors.muted),
            );
            if ui
                .add_enabled(
                    !hs.is_empty(),
                    egui::Button::new("↑ Previous diff · Shift+F7").small(),
                )
                .on_hover_text("Previous hunk (Shift+F7)")
                .clicked()
            {
                self.hunk_cursor = crate::diff_navigation::next(self.hunk_cursor, hs.len(), true);
                self.jump_hunk = self.hunk_cursor;
            }
            if ui
                .add_enabled(
                    !hs.is_empty(),
                    egui::Button::new("↓ Next diff · F7").small(),
                )
                .on_hover_text("Next hunk (F7)")
                .clicked()
            {
                self.hunk_cursor = crate::diff_navigation::next(self.hunk_cursor, hs.len(), false);
                self.jump_hunk = self.hunk_cursor;
            }
            if ui.small_button("Include all").clicked() {
                self.files[i].select(true);
                self.epoch += 1;
            }
            if ui.small_button("Exclude all").clicked() {
                self.files[i].select(false);
                self.epoch += 1;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(if self.saved { "Saved" } else { "Unsaved" })
                        .small()
                        .color(if self.saved {
                            colors.green
                        } else {
                            colors.warn
                        }),
                );
            });
        });
        let ext = file
            .path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_string();
        let mut changed = false;
        let mut revert = None;
        let jump = self.jump_hunk.take();
        // Keep egui's complete accessibility text tree when assistive technology requests it.
        let accessible = ui.ctx().accesskit_node_builder(ui.id(), |_| ()).is_some();
        if !accessible
            && (old.len() + self.draft.len() > 64 * 1024
                || old.lines().count().max(self.draft.lines().count()) > 500)
        {
            let outcome = self.virtual_diff.show(
                ui,
                editor_id,
                &old,
                &mut self.draft,
                &mut self.files[i].hunks,
                &mut self.highlight,
                &ext,
                crate::virtual_diff::Options {
                    width: ui.available_width(),
                    split: self.split_ratio,
                    wrap: self.soft_wrap,
                    unified: self.unified,
                    dark: self.dark,
                },
                jump,
            );
            if let Some(h) = outcome.jump {
                self.hunk_cursor = Some(h);
                self.jump_hunk = Some(h);
                ui.ctx().request_repaint();
            }
            if let Some(split) = outcome.split {
                self.split_ratio = split;
            }
            if outcome.selection_changed {
                self.files[i].selected = self.files[i].hunks.iter().all(|h| h.selected);
                self.epoch += 1;
            }
            let mut changed = outcome.changed;
            if let Some(redo) = history_action
                && let Some(text) = edit_history(ui.ctx(), editor_id, &self.draft, redo)
            {
                self.draft = text;
                changed = true;
            }
            if changed {
                self.save();
            }
            if let Some(h) = outcome.revert {
                self.revert(Some(h));
            }
            return;
        }
        egui::Frame::new().fill(colors.bg).show(ui, |ui| {
            let (content, rail) = crate::diff_navigation::areas(ui.available_rect_before_wrap());
            ui.set_max_width(content.width());
            let viewport_width = ui.available_width();
            let view = DiffLayout::build(
                ui,
                self.highlight.job_for("old", &old, &ext, self.dark),
                self.highlight
                    .job_for("working", &self.draft, &ext, self.dark),
                viewport_width,
                self.unified,
                self.soft_wrap,
                self.split_ratio,
            );
            let mut scroll = if self.soft_wrap {
                egui::ScrollArea::vertical()
            } else {
                egui::ScrollArea::both()
            }
            .id_salt(("diff", &file.path, self.unified, self.soft_wrap))
            .auto_shrink([false, false]);
            if let Some(h) = jump.and_then(|h| view.rows.hunk_rows.get(h)) {
                scroll = scroll.vertical_scroll_offset(view.offsets[h.start]);
            }
            let output = scroll.show(ui, |ui| {
                let ppp = ui.ctx().pixels_per_point();
                let cursor = ui.cursor().min;
                let origin = Pos2::new(
                    diff_view::snap(cursor.x, ppp),
                    diff_view::snap(cursor.y, ppp),
                );
                let height = view.height.max(ui.clip_rect().height() - 28.0).max(120.0);
                let gutter = GUTTER;
                let old_width = view.old_width;
                let new_width = view.new_width;
                let controls_x = if self.unified {
                    origin.x
                } else {
                    origin.x + old_width
                };
                let old_origin = origin + Vec2::new(if self.unified { gutter } else { 0.0 }, 28.0);
                let new_origin = if self.unified {
                    old_origin
                } else {
                    origin + Vec2::new(old_width + gutter, 28.0)
                };
                let old_origin = Pos2::new(
                    diff_view::snap(old_origin.x, ppp),
                    diff_view::snap(old_origin.y, ppp),
                );
                let new_origin = Pos2::new(
                    diff_view::snap(new_origin.x, ppp),
                    diff_view::snap(new_origin.y, ppp),
                );
                if !self.unified {
                    let divider = ui
                        .interact(
                            Rect::from_min_size(
                                Pos2::new(controls_x, origin.y),
                                Vec2::new(gutter, height + 28.0),
                            )
                            .intersect(ui.clip_rect()),
                            egui::Id::new("diff-divider"),
                            egui::Sense::click_and_drag(),
                        )
                        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                        .on_hover_text("Drag to resize panes. Double-click to balance.");
                    if divider.dragged() {
                        let total = old_width + new_width;
                        let delta = ui.input(|input| input.pointer.delta().x);
                        self.split_ratio =
                            diff_view::split_widths(total, (old_width + delta) / total)[0] / total;
                        ui.ctx().request_repaint();
                    }
                    if divider.double_clicked() {
                        self.split_ratio = 0.5;
                    }
                    let center = Pos2::new(controls_x + gutter * 0.5, origin.y + 12.0);
                    for dx in [-4.0, 0.0, 4.0] {
                        ui.painter().vline(
                            center.x + dx,
                            center.y - 4.0..=center.y + 4.0,
                            Stroke::new(
                                1.0_f32,
                                if divider.hovered() || divider.dragged() {
                                    colors.accent
                                } else {
                                    colors.muted
                                },
                            ),
                        );
                    }
                }
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        Pos2::new(controls_x, origin.y + 28.0),
                        Vec2::new(gutter, height),
                    ),
                    0.0,
                    colors.panel,
                );
                ui.allocate_space(Vec2::new(view.width(self.unified), height + 28.0));
                ui.painter().text(
                    old_origin + Vec2::new(8.0, -16.0),
                    egui::Align2::LEFT_CENTER,
                    if self.unified {
                        "WORKING FILE · removed lines are read-only"
                    } else {
                        "HEAD · read-only"
                    },
                    FontId::proportional(11.0),
                    colors.muted,
                );
                if !self.unified {
                    ui.painter().text(
                        new_origin + Vec2::new(8.0, -16.0),
                        egui::Align2::LEFT_CENTER,
                        "WORKING FILE · editable",
                        FontId::proportional(11.0),
                        colors.muted,
                    );
                    ui.painter().vline(
                        new_origin.x,
                        origin.y..=origin.y + height + 28.0,
                        Stroke::new(1.0_f32, colors.border),
                    );
                }
                for (h, range) in view.rows.hunk_rows.iter().enumerate() {
                    let top = old_origin.y + view.offsets[range.start];
                    let bottom = old_origin.y + view.offsets[range.end];
                    if bottom < ui.clip_rect().top() || top > ui.clip_rect().bottom() {
                        continue;
                    }
                    if self.files[i].hunks.get(h).is_some_and(|h| h.selected) {
                        ui.painter().rect_filled(
                            Rect::from_min_max(
                                Pos2::new(controls_x, top),
                                Pos2::new(controls_x + gutter, bottom),
                            ),
                            0.0,
                            colors.included,
                        );
                    }
                    let rect = Rect::from_min_size(
                        Pos2::new(controls_x + 34.0, top),
                        Vec2::new(25.0, view.line_height),
                    );
                    if h < self.files[i].hunks.len() {
                        let selected = &mut self.files[i].hunks[h].selected;
                        let response = ui.put(rect, egui::Checkbox::without_text(selected));
                        if response.changed() {
                            self.files[i].hunks[h].review = false;
                            self.files[i].selected = self.files[i].hunks.iter().all(|h| h.selected);
                            self.epoch += 1;
                        }
                        if self.files[i].hunks[h].review {
                            ui.painter().circle_filled(
                                Pos2::new(controls_x + 59.0, top + 4.0),
                                3.0,
                                colors.warn,
                            );
                        }
                        if ui
                            .put(
                                Rect::from_min_size(
                                    Pos2::new(controls_x + 4.0, top),
                                    Vec2::new(26.0, view.line_height),
                                ),
                                egui::Button::new(">>").frame(false),
                            )
                            .on_hover_text("Revert this hunk (undo available)")
                            .clicked()
                        {
                            revert = Some(h);
                        }
                    }
                    let _ = bottom;
                }
                paint_rows(ui, &view, old_origin, new_origin, self.unified);
                if self.unified {
                    for (row, (old, new)) in view.rows.rows.iter().enumerate() {
                        if old.is_some() && new.is_none() {
                            let rect =
                                diff_view::line_rect(old_origin, old_width, &view.offsets, row);
                            if ui.clip_rect().intersects(rect) {
                                ui.painter()
                                    .with_clip_rect(ui.clip_rect().intersect(rect))
                                    .galley(
                                        old_origin + Vec2::new(TEXT_INSET, 0.0),
                                        view.old.clone(),
                                        ui.visuals().text_color(),
                                    );
                            }
                        }
                    }
                } else {
                    let mut readonly = old.as_str();
                    let mut layouter =
                        |_: &egui::Ui, _: &dyn egui::TextBuffer, _: f32| view.old.clone();
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(Rect::from_min_size(
                            old_origin + Vec2::new(TEXT_INSET, 0.0),
                            Vec2::new(old_width - TEXT_INSET - TEXT_PADDING, height),
                        )),
                        |ui| {
                            ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                            ui.add(
                                egui::TextEdit::multiline(&mut readonly)
                                    .id_salt(("old", &file.path))
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .frame(false)
                                    .margin(Vec2::ZERO)
                                    .desired_width(old_width - TEXT_INSET - TEXT_PADDING)
                                    .layouter(&mut layouter),
                            );
                        },
                    );
                }
                let unified = self.unified;
                let wrap = self.soft_wrap;
                let snapshot = self.draft.clone();
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                    if text.as_str() == snapshot {
                        view.new.clone()
                    } else {
                        DiffLayout::build(
                            ui,
                            self.highlight.job_for("old", &old, &ext, self.dark),
                            self.highlight
                                .job_for("working", text.as_str(), &ext, self.dark),
                            viewport_width,
                            unified,
                            wrap,
                            self.split_ratio,
                        )
                        .new
                    }
                };
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(Rect::from_min_size(
                        new_origin + Vec2::new(TEXT_INSET, 0.0),
                        Vec2::new(new_width - TEXT_INSET - TEXT_PADDING, height),
                    )),
                    |ui| {
                        ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                        changed = ui
                            .add(
                                egui::TextEdit::multiline(&mut self.draft)
                                    .id(editor_id)
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .frame(false)
                                    .margin(Vec2::ZERO)
                                    .desired_width(new_width - TEXT_INSET - TEXT_PADDING)
                                    .layouter(&mut layouter),
                            )
                            .changed();
                    },
                );
            });
            if let Some(h) = crate::diff_navigation::overview(
                ui,
                editor_id.with("change-map"),
                rail,
                &self.files[i].hunks,
                &view.rows.hunk_rows,
                &view.offsets,
                output.content_size.y,
                output.state.offset.y..output.state.offset.y + output.inner_rect.height(),
            ) {
                self.hunk_cursor = Some(h);
                self.jump_hunk = Some(h);
                ui.ctx().request_repaint();
            }
        });
        if let Some(redo) = history_action
            && let Some(text) = edit_history(ui.ctx(), editor_id, &self.draft, redo)
        {
            self.draft = text;
            changed = true;
        }
        if changed {
            self.save();
        }
        if let Some(h) = revert {
            self.revert(Some(h));
        }
    }
    fn accept_whole(&mut self, i: usize, stage: u8) {
        let Some(repo) = &self.repo else {
            return;
        };
        let result = (|| -> anyhow::Result<Option<Vec<u8>>> {
            repo.load_file(&mut self.files[i])?;
            let bytes = repo.conflict_blob(&self.files[i].path, stage)?;
            repo.restore_work(&self.files[i], bytes.as_deref(), &self.files[i].mode)?;
            Ok(bytes)
        })();
        match result {
            Ok(bytes) => {
                let before = self.files[i].exists.then(|| self.files[i].current.clone());
                self.undo.push(Undo {
                    path: self.files[i].path.clone(),
                    before,
                    after: bytes.clone(),
                    before_mode: self.files[i].mode.clone(),
                    after_mode: self.files[i].mode.clone(),
                });
                self.files[i].exists = bytes.is_some();
                self.files[i].current = bytes.unwrap_or_default();
                self.draft = self.files[i].text();
                self.files[i].hunks = model::hunks(&self.files[i].old_text(), &self.draft);
                self.saved = true;
                self.epoch += 1;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    fn resolve_whole(&mut self, i: usize, stage: u8) {
        self.accept_whole(i, stage);
        if self.error.is_none()
            && let Some(r) = &self.repo
        {
            match r.resolve(&self.files[i]) {
                Ok(()) => {
                    self.epoch += 1;
                    self.scan();
                }
                Err(e) => self.error = Some(e.to_string()),
            }
        }
    }
    fn conflict_bar(&mut self, ui: &mut egui::Ui, i: usize) {
        let colors = Colors::new(self.dark);
        let blocks = model::conflicts(&self.draft);
        egui::Frame::new()
            .fill(colors.warning_bg)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} conflict blocks", blocks.len()))
                            .strong()
                            .color(colors.warn),
                    );
                    if ui.button("Use entire ours").clicked() {
                        self.accept_whole(i, 2);
                    }
                    if ui.button("Use entire theirs").clicked() {
                        self.accept_whole(i, 3);
                    }
                    if ui
                        .add_enabled(
                            blocks.is_empty() && self.saved,
                            egui::Button::new("Mark resolved"),
                        )
                        .clicked()
                        && let Some(r) = &self.repo
                    {
                        match r.resolve(&self.files[i]) {
                            Ok(()) => {
                                self.epoch += 1;
                                self.scan();
                                self.notice = "File marked resolved".into();
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                });
                for (n, b) in blocks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("Conflict {}", n + 1));
                        let mut replace = None;
                        if ui.small_button("Accept ours").clicked() {
                            replace = Some(b.ours.clone());
                        }
                        if ui.small_button("Accept theirs").clicked() {
                            replace = Some(b.theirs.clone());
                        }
                        if ui.small_button("Accept both").clicked() {
                            replace = Some(format!("{}{}", b.ours, b.theirs));
                        }
                        if let Some(text) = replace {
                            self.draft.replace_range(b.range.clone(), &text);
                            self.save();
                        }
                    });
                }
                egui::CollapsingHeader::new("Inspect merge inputs").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.source_tab, 0, "Base");
                        ui.selectable_value(&mut self.source_tab, 1, "Ours");
                        ui.selectable_value(&mut self.source_tab, 2, "Theirs");
                    });
                    egui::ScrollArea::both()
                        .id_salt("merge_inputs")
                        .max_height(150.0)
                        .show(ui, |ui| {
                            let mut source = self.conflict_sources[self.source_tab].as_str();
                            ui.add(
                                egui::TextEdit::multiline(&mut source)
                                    .code_editor()
                                    .desired_width(f32::INFINITY),
                            );
                        });
                });
            });
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.confirm_close {
            egui::Window::new("Unsaved edits")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Autosave failed. Closing now discards the editor buffer.");
                    if ui.button("Keep editing").clicked() {
                        self.confirm_close = false;
                    }
                    if ui.button("Discard buffer and close").clicked() {
                        self.saved = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
        }
        if self.open {
            egui::Window::new("Open repository")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Repository directory");
                    ui.add(egui::TextEdit::singleline(&mut self.open_path).desired_width(450.0));
                    ui.horizontal(|ui| {
                        if ui.button("Open in new window").clicked() {
                            match Repo::open(std::path::Path::new(&self.open_path)).and_then(|r| {
                                Ok(std::process::Command::new(std::env::current_exe()?)
                                    .arg(r.root)
                                    .spawn()?)
                            }) {
                                Ok(_) => self.open = false,
                                Err(e) => self.error = Some(e.to_string()),
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.open = false;
                        }
                    });
                });
        }
        if self.confirm_revert {
            egui::Window::new("Revert file?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Restore this file to HEAD, or remove it if unversioned.");
                    ui.label("You can undo this revert while Difly stays open.");
                    ui.horizontal(|ui| {
                        if ui.button("Revert file").clicked() {
                            self.revert(None);
                            self.confirm_revert = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_revert = false;
                        }
                    });
                });
        }
    }
}
fn update_amend_message(
    message: &mut String,
    draft: &mut Option<String>,
    was_amending: bool,
    amending: bool,
    target_message: &str,
) {
    if amending {
        if !was_amending {
            *draft = Some(message.clone());
        }
        *message = target_message.to_owned();
    } else if was_amending {
        *message = draft.take().unwrap_or_default();
    }
}

fn load_dark_theme(storage: Option<&dyn eframe::Storage>) -> bool {
    storage.and_then(|s| s.get_string("theme")).as_deref() == Some("dark")
}

fn store_theme(storage: &mut dyn eframe::Storage, dark: bool) {
    storage.set_string("theme", if dark { "dark" } else { "light" }.into());
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        store_theme(storage, self.dark);
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let colors = Colors::new(self.dark);
        self.poll();
        map_redo_shortcut(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && (self.busy || !self.saved) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.busy {
                self.error = Some("Wait for the commit to finish before closing.".into());
            } else {
                self.confirm_close = true;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            self.commit();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.open = true;
        }
        let previous_theme = self.dark;
        self.header(ctx);
        if self.dark != previous_theme
            && let Some(storage) = frame.storage_mut()
        {
            store_theme(storage, self.dark);
            storage.flush();
        }
        self.sidebar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(colors.bg).inner_margin(12))
            .show(ctx, |ui| {
                ui.add_enabled_ui(!self.busy, |ui| self.editor(ui));
            });
        self.dialogs(ctx);
    }
}
// Route Cmd/Ctrl+R through egui's native redo event, including commit-message
// fields, while keeping Cmd/Ctrl+Z's normal focused-editor behavior.
pub(crate) fn map_redo_shortcut(ctx: &egui::Context) {
    ctx.input_mut(|input| {
        for event in &mut input.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
                && *key == egui::Key::R
                && modifiers.command
                && !modifiers.alt
                && !modifiers.shift
            {
                *key = egui::Key::Z;
                modifiers.shift = true;
            }
        }
    });
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |c: Color32| {
        let linear = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn syntax_color(color: syntect::highlighting::Color, dark: bool) -> Color32 {
    let mut result = Color32::from_rgb(color.r, color.g, color.b);
    if !dark {
        let colors = Colors::new(false);
        while [colors.bg, colors.add, colors.del]
            .iter()
            .any(|&bg| contrast(result, bg) < 4.5)
        {
            result = Color32::from_rgb(
                (result.r() as f32 * 0.9) as u8,
                (result.g() as f32 * 0.9) as u8,
                (result.b() as f32 * 0.9) as u8,
            );
        }
    }
    result
}

/// Use the editor's own history so toolbar actions and keyboard undo stay in sync.
fn edit_history(ctx: &egui::Context, id: egui::Id, text: &str, redo: bool) -> Option<String> {
    let mut state = egui::text_edit::TextEditState::load(ctx, id)?;
    let current = (
        state.cursor.char_range().unwrap_or_default(),
        text.to_owned(),
    );
    let mut history = state.undoer();
    let (cursor, text) = if redo {
        history.redo(&current)
    } else {
        history.undo(&current)
    }?
    .clone();
    state.cursor.set_char_range(Some(cursor));
    state.set_undoer(history);
    state.store(ctx, id);
    Some(text)
}

fn paint_rows(ui: &egui::Ui, view: &DiffLayout, old: Pos2, new: Pos2, unified: bool) {
    let colors = Colors::new(ui.visuals().dark_mode);
    for (row, (a, b)) in view.rows.rows.iter().enumerate() {
        let changed = view.rows.hunk_rows.iter().any(|r| r.contains(&row));
        let panes = if unified {
            vec![(
                old,
                b,
                if b.is_some() { colors.add } else { colors.del },
                view.new_width,
            )]
        } else {
            vec![
                (old, a, colors.del, view.old_width),
                (new, b, colors.add, view.new_width),
            ]
        };
        for (origin, line, fill, width) in panes {
            let rect = diff_view::line_rect(origin, width, &view.offsets, row);
            if !ui.clip_rect().intersects(rect) {
                continue;
            }
            if changed && (line.is_some() || unified) {
                ui.painter().rect_filled(rect, 0.0, fill);
            }
            if line.is_some() || unified {
                let number = line
                    .map(|n| (n + 1).to_string())
                    .unwrap_or_else(|| "−".into());
                ui.painter().text(
                    Pos2::new(origin.x + 37.0, rect.top() + view.line_height * 0.5),
                    egui::Align2::RIGHT_CENTER,
                    number,
                    FontId::monospace(CODE_SIZE),
                    colors.muted,
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_cache_replaces_versions_and_obeys_byte_budget() {
        let mut high = Highlight::new();
        high.budget = 32 * 1024;
        for i in 0..30 {
            high.document(
                "working",
                &format!("{}// version {i}\n", "let n = 42;\n".repeat(100)),
                "rs",
                false,
            );
            assert!(high.cache.len() <= 1);
            assert!(
                high.cache
                    .iter()
                    .map(|(key, d)| key.capacity() + d.bytes())
                    .sum::<usize>()
                    <= high.budget
            );
        }
        high.document("old", "let old = 1;", "rs", false);
        assert_eq!(high.cache.len(), 2);
        high.document(
            "working",
            &"let much_larger = 12;\n".repeat(10_000),
            "rs",
            false,
        );
        assert!(
            high.cache
                .iter()
                .map(|(key, d)| key.capacity() + d.bytes())
                .sum::<usize>()
                <= high.budget
        );
        assert!(high.cache.iter().all(|(key, _)| key != "working"));
    }
    #[test]
    fn visible_syntax_preserves_multiline_state_and_utf8() {
        let mut high = Highlight::new();
        let text = "/* comment starts\nαβ still comment\n*/ let x = 42;\n";
        let doc = high.document("working", text, "rs", false);
        let start = text.find('α').unwrap();
        let end = text.find("*/").unwrap();
        let visible = doc.job(start..end);
        assert_eq!(visible.text, &text[start..end]);
        let whole = doc.job(0..text.len());
        let expected = whole
            .sections
            .iter()
            .find(|s| s.byte_range.contains(&start))
            .unwrap()
            .format
            .color;
        assert!(visible.sections.iter().all(|s| s.format.color == expected));
    }

    #[test]
    fn scrolling_keeps_diff_background_under_visible_text() {
        for wrap in [false, true] {
            let ctx = egui::Context::default();
            apply_theme(&ctx, false);
            let text = (0..50)
                .map(|i| format!("line {i}: {}\n", "a long source line ".repeat(30)))
                .collect::<String>();
            let mut highlighter = Highlight::new();
            let mut scroll_offset = Vec2::ZERO;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let view = DiffLayout::build(
                            ui,
                            highlighter.job("", "rs", false),
                            highlighter.job(&text, "rs", false),
                            ui.available_width(),
                            true,
                            wrap,
                            0.5,
                        );
                        let response = egui::ScrollArea::both()
                            .horizontal_scroll_offset(if wrap { 0.0 } else { 500.0 })
                            .vertical_scroll_offset(500.0)
                            .show(ui, |ui| {
                                let origin = ui.cursor().min;
                                ui.allocate_space(Vec2::new(view.width(true), view.height));
                                paint_rows(ui, &view, origin, origin, true);
                            });
                        scroll_offset = response.state.offset;
                    });
                },
            );
            assert!(scroll_offset.y > 0.0);
            if !wrap {
                assert!(scroll_offset.x > 0.0);
            }
            let viewport_center = Pos2::new(300.0, 200.0);
            assert!(output.shapes.iter().any(|clipped| {
                matches!(&clipped.shape,egui::Shape::Rect(rect)
                    if rect.fill==Colors::new(false).add && rect.rect.intersect(clipped.clip_rect).contains(viewport_center))
            }), "Scrolled text must retain its diff background");
        }
    }

    #[test]
    fn keyboard_undo_and_command_r_redo_use_native_editor_history() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("keyboard-test");
        let mut text = "after".to_string();
        let draw = |ctx: &egui::Context, text: &mut String| {
            map_redo_shortcut(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::multiline(text).id(id));
            });
        };
        let _ = ctx.run(egui::RawInput::default(), |ctx| draw(ctx, &mut text));
        ctx.memory_mut(|m| m.request_focus(id));
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
        let cursor = egui::text::CCursorRange::one(egui::text::CCursor::new(text.chars().count()));
        state.cursor.set_char_range(Some(cursor));
        state.clear_undoer();
        let mut history = state.undoer();
        history.add_undo(&(cursor, "before".to_string()));
        history.add_undo(&(cursor, "after".to_string()));
        state.set_undoer(history);
        state.store(&ctx, id);
        for (key, expected) in [(egui::Key::Z, "before"), (egui::Key::R, "after")] {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| draw(ctx, &mut text));
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn toolbar_undo_and_redo_share_editor_history() {
        let ctx = egui::Context::default();
        let id = egui::Id::new(("working", std::path::Path::new("file.rs")));
        let mut state = egui::text_edit::TextEditState::default();
        let cursor = egui::text::CCursorRange::default();
        let mut history = state.undoer();
        history.add_undo(&(cursor, "before".to_string()));
        history.add_undo(&(cursor, "after".to_string()));
        state.set_undoer(history);
        state.store(&ctx, id);
        assert_eq!(
            edit_history(&ctx, id, "after", false).as_deref(),
            Some("before")
        );
        assert_eq!(
            edit_history(&ctx, id, "before", true).as_deref(),
            Some("after")
        );
        assert_eq!(edit_history(&ctx, id, "after", true), None);
    }

    #[test]
    fn leaving_amend_restores_original_draft_across_target_changes() {
        let mut message = "My draft\n\nDetails".to_owned();
        let mut draft = None;
        update_amend_message(&mut message, &mut draft, false, true, "last commit");
        assert_eq!(message, "last commit");
        message.push_str(" edited");
        update_amend_message(&mut message, &mut draft, true, true, "older commit");
        assert_eq!(message, "older commit");
        update_amend_message(&mut message, &mut draft, true, false, "");
        assert_eq!(message, "My draft\n\nDetails");
        assert!(draft.is_none());
        message.clear();
        update_amend_message(&mut message, &mut draft, false, true, "last commit");
        update_amend_message(&mut message, &mut draft, true, false, "");
        assert!(message.is_empty());
    }

    #[test]
    fn theme_preference_round_trips_and_defaults_to_light() {
        #[derive(Default)]
        struct Storage(HashMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn flush(&mut self) {}
        }
        let mut storage = Storage::default();
        assert!(!load_dark_theme(None));
        assert!(!load_dark_theme(Some(&storage)));
        store_theme(&mut storage, true);
        assert!(load_dark_theme(Some(&storage)));
        store_theme(&mut storage, false);
        assert!(!load_dark_theme(Some(&storage)));
        eframe::Storage::set_string(&mut storage, "theme", "invalid".into());
        assert!(!load_dark_theme(Some(&storage)));
        assert!(contrast(Colors::new(false).accent, Color32::from_rgb(220, 232, 252)) >= 4.5);
    }

    #[test]
    fn light_text_meets_contrast_on_diff_backgrounds() {
        let colors = Colors::new(false);
        let mut highlighter = Highlight::new();
        for (ext, text) in [
            ("rs", "// comment\nlet value = 42; println!(\"hello\");"),
            ("yml", "name: Rust\non: [push, pull_request]\n"),
        ] {
            for section in highlighter.job(text, ext, false).sections {
                for bg in [colors.bg, colors.add, colors.del] {
                    assert!(contrast(section.format.color, bg) >= 4.5);
                }
            }
        }
        assert!(contrast(colors.muted, colors.panel) >= 4.5);
        assert!(contrast(colors.accent, colors.panel) >= 4.5);
    }

    #[test]
    fn light_theme_and_syntax_cache_are_independent_of_dark_theme() {
        let ctx = egui::Context::default();
        apply_theme(&ctx, false);
        assert!(!ctx.style().visuals.dark_mode);
        assert_eq!(ctx.style().visuals.extreme_bg_color, Colors::new(false).bg);
        let mut highlighter = Highlight::new();
        let light = highlighter.job("let answer = 42;", "rs", false);
        let dark = highlighter.job("let answer = 42;", "rs", true);
        assert_ne!(
            light.sections[0].format.color,
            dark.sections[0].format.color
        );
        assert_eq!(light, highlighter.job("let answer = 42;", "rs", false));
        apply_theme(&ctx, true);
        assert!(ctx.style().visuals.dark_mode);
    }

    #[test]
    fn editor_layout_preserves_text_and_maps_rows_in_both_views() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        apply_theme(&ctx, false);
        let mut highlighter = Highlight::new();
        let old = "let a = 1;\nold\nlast\n";
        let current = "// inserted\nlet a = 1;\nlast\n";
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for unified in [false, true] {
                    let view = DiffLayout::build(
                        ui,
                        highlighter.job(old, "rs", false),
                        highlighter.job(current, "rs", false),
                        800.0,
                        unified,
                        false,
                        0.5,
                    );
                    let galley = &view.new;
                    assert_eq!(galley.job.text, current);
                    assert_eq!(galley.rows.len(), current.lines().count() + 1);
                    for row in &galley.rows {
                        let physical_y = row.pos.y * ui.ctx().pixels_per_point();
                        assert!((physical_y - physical_y.round()).abs() < 0.001);
                    }
                    assert!(galley.rows.windows(2).all(|w| w[0].pos.y < w[1].pos.y));
                }
            });
        });
    }
}
