mod file_tree;
mod indentation;
mod language;
#[cfg(feature = "ui-tests")]
#[allow(dead_code)]
pub mod native_tests;
mod presentation;
mod view;
mod workflow;
use difly::session::{Session, byte_range};
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    diff::{Diff, DiffFile, DiffLinePosition, DiffSide, DiffState},
    input::{
        Editor, EditorState, InputEvent, OverviewRulerDecoration,
        OverviewRulerDecorationCollection, Position, TabSize, TextDecorationCollection,
    },
    list::ListItem,
    resizable::{h_resizable, resizable_panel},
    tree::{TreeState, tree},
};
use gpui_kit::{prelude::*, *};
use std::{collections::HashMap, path::PathBuf};

gpui_kit::actions!(
    difly_gpui,
    [
        NextHunk,
        PreviousHunk,
        ToggleTreeInclusion,
        Commit,
        OpenRepository,
        Quit
    ]
);

pub struct DiffApp {
    session: Session,
    old: Entity<EditorState>,
    working: Entity<EditorState>,
    tree: Entity<TreeState>,
    tree_members: HashMap<String, Vec<usize>>,
    gutter_bands: Vec<(usize, f32, f32)>,
    gutter_bounds: Option<Bounds<Pixels>>,
    branch: String,
    dark: bool,
    overview: OverviewRulerDecorationCollection,
    message: Entity<gpui_kit::component::input::TextareaState>,
    filter: Entity<gpui_kit::component::input::InputState>,
    settings: difly::settings::Settings,
    settings_path: PathBuf,
    busy: bool,
    dialog_open: bool,
    unified: bool,
    review: Entity<DiffState>,
    indent_marks: [TextDecorationCollection; 2],
    _subscriptions: Vec<Subscription>,
    status: SharedString,
    wrap: bool,
    save_failed: bool,
    linked_scroll: bool,
    scroll_sync: presentation::ScrollSync,
}
impl DiffApp {
    pub fn new(
        mut session: Session,
        settings: difly::settings::Settings,
        settings_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if !session.repo.merge()
            && let Some(message) = settings.drafts.get(&session.repo.root)
        {
            session.draft.message = message.clone();
        }
        let message = cx.new(|cx| {
            gpui_kit::component::input::TextareaState::new(window, cx)
                .default_value(session.draft.message.clone())
                .placeholder("Describe why this change matters…")
        });
        let filter = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx).placeholder("Filter files…")
        });
        let review = cx.new(|cx| DiffState::new([], cx));
        let old = cx.new(|cx| {
            EditorState::new(window, cx)
                .line_number(true)
                .soft_wrap(settings.soft_wrap)
        });
        let working = cx.new(|cx| {
            EditorState::new(window, cx)
                .line_number(true)
                .soft_wrap(settings.soft_wrap)
        });
        let overview = working.update(cx, |s, cx| {
            s.create_overview_ruler_decorations_collection(vec![], cx)
        });
        let indent_marks = [
            old.update(cx, |s, cx| s.create_decorations_collection(vec![], cx)),
            working.update(cx, |s, cx| s.create_decorations_collection(vec![], cx)),
        ];
        let mut subscriptions =
            vec![
                cx.subscribe_in(&working, window, |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = this.working.read(cx).value();
                        match this.session.save(value.as_ref()) {
                            Ok(()) => {
                                this.save_failed = false;
                                this.status = "Saved".into();
                                this.refresh_marks(cx);
                            }
                            Err(e) => {
                                this.save_failed = true;
                                this.status =
                                    format!("Save failed: {e}. Your draft remains in this editor.")
                                        .into();
                            }
                        }
                        cx.notify();
                    }
                }),
            ];
        subscriptions.push(cx.subscribe_in(
            &message,
            window,
            |this, message, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.session.draft.message = message.read(cx).value().to_string();
                    this.persist();
                    cx.notify();
                }
                InputEvent::PressEnter {
                    secondary: true, ..
                } => this.commit(window, cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &filter,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild_tree(cx);
                    cx.notify();
                }
            },
        ));
        for source in [&old, &working] {
            subscriptions.push(cx.observe_in(source, window, |_, _, _, cx| cx.notify()));
        }
        let (items, tree_members) = file_tree::build(&session.files, settings.group_folders, "");
        let tree = cx.new(|cx| TreeState::new(cx).items(items));
        subscriptions.push(cx.observe_in(&tree, window, |this, state, window, cx| {
            let id = state
                .read(cx)
                .selected_item()
                .map(|item| item.id.to_string());
            if let Some(index) = id.and_then(|id| {
                id.strip_prefix("file:")
                    .and_then(|s| s.parse::<usize>().ok())
            }) && this.session.active != Some(index)
            {
                this.activate(index, window, cx);
            }
        }));
        let branch = session.repo.branch();
        let mut app = Self {
            session,
            tree,
            tree_members,
            gutter_bands: vec![],
            gutter_bounds: None,
            branch,
            dark: settings.dark,
            old,
            working,
            overview,
            message,
            filter,
            settings: settings.clone(),
            settings_path,
            busy: false,
            dialog_open: false,
            unified: false,
            review,
            indent_marks,
            _subscriptions: subscriptions,
            status: "Ready".into(),
            wrap: settings.soft_wrap,
            save_failed: false,
            linked_scroll: true,
            scroll_sync: presentation::ScrollSync::default(),
        };
        if !app.session.files.is_empty() {
            app.activate(0, window, cx);
            app.working.update(cx, |s, cx| s.focus(window, cx));
        }
        app
    }
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_failed {
            self.status = "Cannot switch files while the current draft has a save error.".into();
            cx.notify();
            return;
        }
        if let Err(e) = self.session.activate(index) {
            self.status = format!("Open failed: {e}").into();
            cx.notify();
            return;
        }
        let file = self.session.file().unwrap();
        let language = language::detect(&file.path, &file.text());
        let before = file.old_text();
        let after = file.text();
        let editable = !file.binary && file.exists;
        self.old.update(cx, |s, cx| {
            s.set_tab_size(
                TabSize {
                    tab_size: indentation::width(&before),
                    hard_tabs: false,
                },
                cx,
            );
            s.set_value(before, window, cx);
            s.set_highlighter(language.clone(), cx);
        });
        self.working.update(cx, |s, cx| {
            s.set_tab_size(
                TabSize {
                    tab_size: indentation::width(&after),
                    hard_tabs: false,
                },
                cx,
            );
            s.set_value(after, window, cx);
            s.set_highlighter(language.clone(), cx);
        });
        self.status = if editable {
            "Loaded · edits autosave"
        } else {
            "Read-only preview (binary or deleted file)"
        }
        .into();
        self.tree.update(cx, |s, cx| {
            s.set_selected_item(
                Some(&gpui_kit::component::tree::TreeItem::new(
                    format!("file:{index}"),
                    "",
                )),
                cx,
            )
        });
        self.gutter_bands.clear();
        self.refresh_marks(cx);
        cx.notify();
    }
    fn sync_scroll(&mut self, cx: &mut Context<Self>) {
        let editors = [self.old.clone(), self.working.clone()];
        let offsets = [
            editors[0].read(cx).scroll_offset(),
            editors[1].read(cx).scroll_offset(),
        ];
        if self.linked_scroll
            && let Some((target, offset)) = self.scroll_sync.update(offsets)
        {
            editors[target].update(cx, |s, cx| s.set_scroll_offset(offset, cx));
        }
    }
    fn refresh_marks(&mut self, cx: &mut Context<Self>) {
        let mut map = vec![];
        if let Some(file) = self.session.file() {
            let after = file.text();
            for hunk in &file.hunks {
                let b = byte_range(&after, hunk.new.clone());
                map.push(
                    OverviewRulerDecoration::new(b).with_color(
                        rgb(if hunk.new.is_empty() {
                            0xC74747
                        } else if hunk.old.is_empty() {
                            0x279D51
                        } else {
                            0x4487DA
                        })
                        .into(),
                    ),
                );
            }
        }
        self.overview.set(map, cx);
        let files = self
            .session
            .file()
            .filter(|f| !f.binary)
            .map(|f| {
                let patch = similar::TextDiff::from_lines(&f.old_text(), &f.text())
                    .unified_diff()
                    .header(
                        &format!("a/{}", f.path.display()),
                        &format!("b/{}", f.path.display()),
                    )
                    .to_string();
                match DiffFile::parse(&patch) {
                    Ok(files) => files,
                    Err(error) => {
                        self.status = format!("Unified review unavailable: {error}").into();
                        vec![]
                    }
                }
            })
            .unwrap_or_default();
        self.review.update(cx, |s, cx| s.set_files(files, cx));
        self.refresh_indentation(cx);
    }
    fn refresh_indentation(&self, cx: &mut Context<Self>) {
        for (editor, marks) in [
            (&self.old, &self.indent_marks[0]),
            (&self.working, &self.indent_marks[1]),
        ] {
            marks.set(
                indentation::decorations(editor.read(cx).value().as_ref(), self.dark),
                cx,
            );
        }
    }
    fn jump(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((a, b)) = self.session.jump(backwards) {
            if self.unified {
                if let Some(file) = self.session.file() {
                    let line = file.text()[..b.start]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count()
                        + 1;
                    let path = file.path.to_string_lossy().into_owned();
                    self.review.update(cx, |s, cx| {
                        s.scroll_to_line(DiffLinePosition::new(path, DiffSide::Modified, line), cx)
                    });
                }
                cx.notify();
                return;
            }
            for (editor, range) in [(self.old.clone(), a), (self.working.clone(), b)] {
                editor.update(cx, |s, cx| {
                    let text = s.value();
                    let prefix = &text[..range.start];
                    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
                    s.set_cursor_position(Position::new(line, 0), window, cx);
                    s.set_selected_range(range, cx);
                });
            }
        }
        cx.notify();
    }
    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value();
        let (items, members) = file_tree::build(
            &self.session.files,
            self.settings.group_folders,
            query.as_ref(),
        );
        self.tree_members = members;
        self.tree.update(cx, |s, cx| s.set_items(items, cx));
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.dialog_open {
            return;
        }
        if self.save_failed {
            self.dialog_open = true;
            let result = window.prompt(
                PromptLevel::Warning,
                "Reload from disk?",
                Some("Autosave failed. Reloading discards the unsaved editor buffer."),
                &["Keep editing", "Discard buffer and reload"],
                cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                let answer = result.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.dialog_open = false;
                    if answer == Ok(1) {
                        this.refresh_from_disk(window, cx);
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        self.refresh_from_disk(window, cx);
    }
    fn refresh_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.session.file().map(|f| f.path.clone());
        match self.session.refresh() {
            Ok(()) => {
                self.save_failed = false;
                if let Some(i) =
                    path.and_then(|p| self.session.files.iter().position(|f| f.path == p))
                {
                    self.session.active = Some(i);
                }
                self.reload_view(window, cx);
            }
            Err(e) => self.status = format!("Refresh failed: {e}").into(),
        }
        cx.notify();
    }
    fn revert(&mut self, hunk: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_failed {
            return;
        }
        match self.session.revert(hunk) {
            Ok(_) => {
                self.reload_view(window, cx);
                self.status = "Hunk reverted · saved".into();
                self.refresh_marks(cx);
            }
            Err(e) => self.status = format!("Revert failed: {e}").into(),
        }
        cx.notify();
    }
}

pub fn bind_keys(cx: &mut App) {
    let (commit, open, redo) = if cfg!(target_os = "macos") {
        ("cmd-enter", "cmd-o", "cmd-r")
    } else {
        ("ctrl-enter", "ctrl-o", "ctrl-r")
    };
    cx.set_menus([
        Menu {
            name: "Difly".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Open repository…", OpenRepository),
                MenuItem::separator(),
                MenuItem::action("Quit Difly", Quit),
            ],
        },
        Menu {
            name: "Edit".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Undo", gpui_kit::component::input::Undo),
                MenuItem::action("Redo", gpui_kit::component::input::Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", gpui_kit::component::input::Cut),
                MenuItem::action("Copy", gpui_kit::component::input::Copy),
                MenuItem::action("Paste", gpui_kit::component::input::Paste),
                MenuItem::action("Select all", gpui_kit::component::input::SelectAll),
            ],
        },
    ]);
    cx.bind_keys([
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            },
            Quit,
            Some("DiflyDiff"),
        ),
        KeyBinding::new("space", ToggleTreeInclusion, Some("Tree")),
        KeyBinding::new("f7", NextHunk, Some("DiflyDiff")),
        KeyBinding::new("shift-f7", PreviousHunk, Some("DiflyDiff")),
        KeyBinding::new(commit, Commit, Some("DiflyDiff")),
        KeyBinding::new(open, OpenRepository, Some("DiflyDiff")),
        KeyBinding::new(redo, gpui_kit::component::input::Redo, Some("Input")),
    ]);
}
