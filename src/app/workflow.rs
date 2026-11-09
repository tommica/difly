//! Commands that connect the toolkit-independent session to GPUI.
use super::*;
use gpui_kit::component::{
    input::Textarea,
    menu::{DropdownMenu, PopupMenuItem},
};

impl DiffApp {
    pub(super) fn persist(&mut self) {
        self.settings.dark = self.dark;
        self.settings.soft_wrap = self.wrap;
        let message = self.session.draft.normal_message().to_owned();
        if message.is_empty() {
            self.settings.drafts.remove(&self.session.repo.root);
        } else {
            self.settings
                .drafts
                .insert(self.session.repo.root.clone(), message);
        }
        if let Err(e) = self.settings.save(&self.settings_path) {
            self.status = format!("Could not save preferences: {e}").into();
        }
    }
    pub(super) fn can_commit(&self) -> bool {
        !self.busy
            && !self.save_failed
            && !self.session.draft.message.trim().is_empty()
            && (self.session.repo.merge()
                || self.session.draft.target.is_some()
                || self.session.files.iter().any(|f| f.included()))
            && !self.session.files.iter().any(|f| f.conflict)
    }
    pub(super) fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_commit() {
            return;
        }
        let request = match self.session.commit_request() {
            Ok(request) => request,
            Err(e) => {
                self.status = e.to_string().into();
                cx.notify();
                return;
            }
        };
        self.busy = true;
        self.status = "Committing…".into();
        cx.notify();
        let job = cx
            .background_executor()
            .spawn(async move { request.execute() });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(oid) => {
                        this.session.draft = Default::default();
                        this.message.update(cx, |s, cx| s.set_value("", window, cx));
                        if let Err(e) = this.session.committed() {
                            this.status =
                                format!("Commit {oid} created; refresh failed: {e}").into();
                        } else {
                            this.reload_view(window, cx);
                            this.status = format!("Committed {}", &oid[..oid.len().min(8)]).into();
                        }
                        this.persist();
                    }
                    Err(e) => this.status = format!("Commit failed: {e:#}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn amend(
        &mut self,
        target: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.session
            .draft
            .amend(target.and_then(|i| self.session.recent_commits.get(i)));
        let message = self.session.draft.message.clone();
        self.message
            .update(cx, |s, cx| s.set_value(message, window, cx));
        self.persist();
        cx.notify();
    }
    pub(super) fn open_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_failed {
            self.status = "Save the current draft before opening another repository".into();
            cx.notify();
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let result = Session::open(path);
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.busy || this.save_failed {
                        return;
                    }
                    match result {
                        Ok(mut session) => {
                            this.persist();
                            if !session.repo.merge()
                                && let Some(message) = this.settings.drafts.get(&session.repo.root)
                            {
                                session.draft.message = message.clone();
                            }
                            this.session = session;
                            let message = this.session.draft.message.clone();
                            this.message
                                .update(cx, |s, cx| s.set_value(message, window, cx));
                            this.reload_view(window, cx);
                            this.status = "Ready".into();
                        }
                        Err(e) => this.status = format!("Open failed: {e}").into(),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
    pub(super) fn reload_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rebuild_tree(cx);
        self.branch = self.session.repo.branch();
        self.gutter_bands.clear();
        let index = self.session.active.unwrap_or(0);
        if !self.session.files.is_empty() {
            self.activate(index, window, cx);
        } else {
            self.old.update(cx, |s, cx| s.set_value("", window, cx));
            self.working.update(cx, |s, cx| s.set_value("", window, cx));
            self.refresh_marks(cx);
        }
    }
    pub(super) fn revert_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_failed || self.session.file().is_none() || self.dialog_open {
            return;
        }
        self.dialog_open = true;
        let answer = window.prompt(PromptLevel::Warning, "Revert this file?", Some("Restore the file to HEAD, or remove it if unversioned. Undo revert will be available."), &["Cancel", "Revert file"], cx);
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.dialog_open = false;
                if answer == Ok(1) {
                    match this.session.revert_file() {
                        Ok(()) => {
                            this.reload_view(window, cx);
                            this.status = "File reverted".into();
                        }
                        Err(e) => this.status = format!("Revert failed: {e}").into(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn undo_revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_failed {
            return;
        }
        match self.session.undo_revert() {
            Ok(()) => {
                self.reload_view(window, cx);
                self.status = "Revert undone".into();
            }
            Err(e) => self.status = format!("Undo failed: {e}").into(),
        }
        cx.notify();
    }
    pub(super) fn resolve(
        &mut self,
        stage: Option<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.save_failed {
            return;
        }
        let result = match stage {
            Some(stage) => self.session.accept_conflict(stage),
            None => self.session.resolve(),
        };
        match result {
            Ok(()) => {
                self.reload_view(window, cx);
                self.status = if stage.is_some() {
                    "Review the result, then mark resolved"
                } else {
                    "Conflict resolved"
                }
                .into();
            }
            Err(e) => self.status = format!("Resolve failed: {e}").into(),
        }
        cx.notify();
    }
    pub(super) fn review_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.session.file().map_or(0, |f| f.hunks.len());
        let controls: Vec<_> = (0..count)
            .map(|h| {
                let selected = self.session.file().unwrap().hunks[h].selected;
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Checkbox::new(("review-hunk", h))
                            .label(format!("Hunk {}", h + 1))
                            .xsmall()
                            .checked(selected)
                            .disabled(self.busy)
                            .on_click(cx.listener(move |this, selected: &bool, _, cx| {
                                if let Err(e) = this.session.select_hunk(h, *selected) {
                                    this.status = e.to_string().into();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("review-revert", h))
                            .label(">>")
                            .xsmall()
                            .ghost()
                            .disabled(self.busy || self.save_failed)
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.revert(h, window, cx)),
                            ),
                    )
            })
            .collect();
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .px_3()
            .pb_2()
            .children(controls)
    }
    pub(super) fn conflict_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let conflict = self.session.file().is_some_and(|f| f.conflict);
        let unresolved = self
            .session
            .file()
            .map(|f| difly::model::conflicts(&f.text()).len())
            .unwrap_or_default();
        div().when(conflict, |d| {
            d.flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .bg(rgba(0xD69E2030))
                .child(format!("{unresolved} conflict blocks"))
                .child(
                    Button::new("ours")
                        .label("Use entire ours")
                        .xsmall()
                        .disabled(self.busy || self.save_failed)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.resolve(Some(2), window, cx)),
                        ),
                )
                .child(
                    Button::new("theirs")
                        .label("Use entire theirs")
                        .xsmall()
                        .disabled(self.busy || self.save_failed)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.resolve(Some(3), window, cx)),
                        ),
                )
                .child(
                    Button::new("resolved")
                        .label("Mark resolved")
                        .xsmall()
                        .disabled(self.busy || self.save_failed || unresolved > 0)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.resolve(None, window, cx)),
                        ),
                )
        })
    }
    pub fn guard_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            self.status = "Wait for the commit to finish before closing".into();
            cx.notify();
            return false;
        }
        self.persist();
        if !self.save_failed {
            return true;
        }
        if self.dialog_open {
            return false;
        }
        self.dialog_open = true;
        let result = window.prompt(
            PromptLevel::Warning,
            "Autosave failed",
            Some("Closing now discards the unsaved editor buffer."),
            &["Keep editing", "Discard buffer and close"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = result.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.dialog_open = false;
                if answer == Ok(1) {
                    cx.quit();
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
        false
    }
    pub(super) fn commit_panel(
        &self,
        palette: super::view::Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        let commits = self.session.recent_commits.clone();
        let target = self.session.draft.target.clone();
        let label = target
            .as_ref()
            .and_then(|id| commits.iter().find(|c| &c.oid == id))
            .map(|c| c.subject())
            .unwrap_or("last commit")
            .to_owned();
        let amended = self.session.draft.target.is_some();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(palette.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Checkbox::new("amend")
                            .label("Amend")
                            .xsmall()
                            .checked(amended)
                            .disabled(self.busy || self.session.repo.merge() || commits.is_empty())
                            .on_click(cx.listener(|this, checked: &bool, window, cx| {
                                this.amend(checked.then_some(0), window, cx)
                            })),
                    )
                    .child(
                        Button::new("amend-target")
                            .label(label)
                            .xsmall()
                            .ghost()
                            .disabled(self.busy || self.session.repo.merge())
                            .dropdown_menu(move |mut menu, _, _| {
                                for (i, commit) in commits.iter().enumerate() {
                                    let view = view.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(commit.subject().to_owned())
                                            .checked(target.as_ref() == Some(&commit.oid))
                                            .on_click(move |_, window, cx| {
                                                view.update(cx, |this, cx| {
                                                    this.amend(Some(i), window, cx)
                                                })
                                            }),
                                    );
                                }
                                menu
                            }),
                    ),
            )
            .child(
                Textarea::new(&self.message)
                    .h(px(110.))
                    .disabled(self.busy)
                    .aria_label("Commit message"),
            )
            .child(
                gpui_kit::base::Button::new("commit")
                    .disabled(!self.can_commit())
                    .accessibility_label(if amended { "Amend commit" } else { "Commit" })
                    .w_full()
                    .h(px(34.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(0x315ACC))
                    .bg(rgb(if self.dark { 0x263C62 } else { 0xDCE8FC }))
                    .text_color(rgb(if self.dark { 0xC8DAFF } else { 0x214FA6 }))
                    .child(
                        div()
                            .when(!self.can_commit(), |d| d.line_through())
                            .child(if amended { "Amend commit" } else { "Commit" }),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(palette.muted)
                    .child("Remaining changes stay unstaged."),
            )
    }
}
