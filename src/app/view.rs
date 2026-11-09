use super::*;
use presentation::{CODE_SIZE, ROW_HEIGHT, backgrounds, clip_band, vertical_range};

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub(super) panel: Hsla,
    pub(super) border: Hsla,
    pub(super) muted: Hsla,
    included: Hsla,
    added: Hsla,
    removed: Hsla,
}
impl Palette {
    fn new(dark: bool) -> Self {
        let colors = if dark {
            [0x202631, 0x3B4555, 0xA7B3C5, 0x51416D, 0x263F32, 0x482F36]
        } else {
            [0xEEF1F6, 0xC8D0DD, 0x56657A, 0xD9CDF8, 0xD5EBD7, 0xF1DCDC]
        };
        Self {
            panel: rgb(colors[0]).into(),
            border: rgb(colors[1]).into(),
            muted: rgb(colors[2]).into(),
            included: rgb(colors[3]).into(),
            added: rgb(colors[4]).into(),
            removed: rgb(colors[5]).into(),
        }
    }
}
impl DiffApp {
    fn update_gutter(&mut self, cx: &mut Context<Self>) {
        let state = self.working.read(cx);
        let Some(bounds) = self.gutter_bounds else {
            return;
        };
        let bands: Vec<_> = self
            .session
            .file()
            .into_iter()
            .flat_map(|f| f.hunks.iter().enumerate())
            .filter_map(|(h, chunk)| {
                let (top, bottom) = vertical_range(state, &chunk.new)?;
                let (top, height) = clip_band(
                    f32::from(top - bounds.origin.y),
                    f32::from(bottom - bounds.origin.y),
                    f32::from(bounds.size.height),
                )?;
                Some((h, top, height))
            })
            .collect();
        if self.gutter_bands != bands {
            self.gutter_bands = bands;
            cx.notify();
        }
    }
    fn file_tree(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        tree(&self.tree, move |index, entry, _, _, cx| {
            view.update(cx, |this, cx| {
                let id = entry.item().id.to_string();
                let indices = this.tree_members.get(&id).cloned().unwrap_or_default();
                let selection = file_tree::selection_state(&this.session.files, &indices);
                let file = (!entry.is_folder()).then(|| &this.session.files[indices[0]]);
                let status = file.and_then(file_tree::status_label);
                let counts = file.and_then(|f| f.line_changes());
                let added_color = rgb(if this.dark { 0x82D9A0 } else { 0x226C3C });
                let removed_color = rgb(if this.dark { 0xFF9D9D } else { 0xAD3434 });
                let checkbox_view = cx.entity();
                let active = selection != gpui_kit::base::CheckboxState::Unchecked;
                let indicator = div()
                    .size(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_color(if active {
                        cx.theme().primary
                    } else {
                        palette.border
                    })
                    .bg(if active {
                        cx.theme().primary
                    } else {
                        cx.theme().background
                    })
                    .text_color(cx.theme().primary_foreground)
                    .when(active, |d| {
                        d.child(
                            gpui_kit::component::Icon::new(
                                if selection == gpui_kit::base::CheckboxState::Indeterminate {
                                    IconName::Minus
                                } else {
                                    IconName::Check
                                },
                            )
                            .xsmall(),
                        )
                    });
                let icon = if !entry.is_folder() {
                    IconName::File
                } else if entry.is_expanded() {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                };
                let arrow = if !entry.is_folder() {
                    " "
                } else if entry.is_expanded() {
                    "⌄"
                } else {
                    "›"
                };
                ListItem::new(index)
                    .h(px(29.))
                    .px_2()
                    .pl(px(8. + entry.depth() as f32 * 18.))
                    .rounded_sm()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .w_full()
                            .min_w_0()
                            .text_size(px(13.))
                            .child(div().w(px(10.)).text_color(palette.muted).child(arrow))
                            .child(
                                gpui_kit::base::Checkbox::new(("include-file", index))
                                    .state(selection)
                                    .disabled(this.busy)
                                    .tab_stop(false)
                                    .accessibility_label(format!("Include {}", entry.item().label))
                                    .child(indicator)
                                    .on_change(move |next, _, _, cx| {
                                        checkbox_view.update(cx, |this, cx| {
                                            for &i in &indices {
                                                this.session.files[i].select(
                                                    next == gpui_kit::base::CheckboxState::Checked,
                                                );
                                            }
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(icon)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(entry.item().label.clone()),
                            )
                            .when_some(status, |row, status| {
                                row.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(11.))
                                        .text_color(
                                            if status == "Deleted" || status == "Conflict" {
                                                removed_color
                                            } else {
                                                added_color
                                            },
                                        )
                                        .child(status),
                                )
                            })
                            .when_some(counts, |row, (added, removed)| {
                                row.child(
                                    div()
                                        .flex()
                                        .flex_shrink_0()
                                        .gap_1()
                                        .text_size(px(11.))
                                        .child(
                                            div()
                                                .text_color(added_color)
                                                .child(format!("+{added}")),
                                        )
                                        .child(
                                            div()
                                                .text_color(removed_color)
                                                .child(format!("−{removed}")),
                                        ),
                                )
                            }),
                    )
            })
        })
        .text_size(px(13.))
        .p_1()
    }
    fn gutter(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let hunks = self
            .session
            .file()
            .map(|f| f.hunks.as_slice())
            .unwrap_or_default();
        let bands: Vec<_> = self
            .gutter_bands
            .iter()
            .filter_map(|&(h, top, height)| {
                let chunk = hunks.get(h)?;
                Some(
                    div()
                        .absolute()
                        .top(px(top))
                        .h(px(height))
                        .w_full()
                        .bg(if chunk.selected {
                            palette.included
                        } else {
                            palette.panel
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .h(px(ROW_HEIGHT))
                                .justify_between()
                                .px_0p5()
                                .child(
                                    Button::new(("revert", h))
                                        .label(">>")
                                        .ghost()
                                        .xsmall()
                                        .w(px(27.))
                                        .disabled(self.busy || self.save_failed)
                                        .tooltip(format!("Revert hunk {}", h + 1))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.revert(h, window, cx)
                                        })),
                                )
                                .child(
                                    Checkbox::new(("hunk", h))
                                        .xsmall()
                                        .tab_stop(false)
                                        .checked(chunk.selected)
                                        .disabled(self.busy)
                                        .accessibility_label(format!("Include hunk {}", h + 1))
                                        .on_click(cx.listener(
                                            move |this, checked: &bool, _, cx| {
                                                if !this.busy
                                                    && let Err(e) =
                                                        this.session.select_hunk(h, *checked)
                                                {
                                                    this.status = e.to_string().into();
                                                }
                                                cx.notify();
                                            },
                                        )),
                                ),
                        ),
                )
            })
            .collect();
        let view = cx.weak_entity();
        let measure = canvas(
            move |bounds, window, cx| {
                let view = view.clone();
                window.defer(cx, move |_, cx| {
                    let _ = view.update(cx, |this, cx| {
                        if this.gutter_bounds != Some(bounds) {
                            this.gutter_bounds = Some(bounds);
                            cx.notify();
                        }
                    });
                });
            },
            |_, _, _, _| (),
        )
        .absolute()
        .size_full();
        div()
            .relative()
            .flex_shrink_0()
            .w(px(58.))
            .h_full()
            .overflow_hidden()
            .border_l_1()
            .border_r_1()
            .border_color(palette.border)
            .bg(palette.panel)
            .child(measure)
            .children(bands)
    }
    fn pane(&self, old: bool, palette: Palette, cx: &App) -> impl IntoElement {
        let editor = if old {
            self.old.clone()
        } else {
            self.working.clone()
        };
        let ranges = self
            .session
            .file()
            .map(|f| {
                f.hunks
                    .iter()
                    .map(|h| if old { h.old.clone() } else { h.new.clone() })
                    .collect()
            })
            .unwrap_or_default();
        let editable = self.session.file().is_some_and(|f| !f.binary && f.exists);
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(backgrounds(
                editor.clone(),
                ranges,
                if old { palette.removed } else { palette.added },
            ))
            .child(
                Editor::new(&editor)
                    .appearance(false)
                    .bordered(false)
                    .readonly(old || !editable)
                    .disabled(self.busy)
                    .h(relative(1.))
                    .text_size(px(CODE_SIZE))
                    .font_family(cx.theme().mono_font_family.clone()),
            )
    }
    fn toolbar(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.session.file().map_or(0, |f| f.hunks.len());
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .h(px(36.))
            .px_3()
            .child(
                Button::new("previous")
                    .label("↑ Previous")
                    .ghost()
                    .xsmall()
                    .disabled(count == 0)
                    .tooltip("Previous diff · Shift+F7")
                    .on_click(cx.listener(|this, _, window, cx| this.jump(true, window, cx))),
            )
            .child(
                Button::new("next")
                    .label("↓ Next")
                    .ghost()
                    .xsmall()
                    .disabled(count == 0)
                    .tooltip("Next diff · F7")
                    .on_click(cx.listener(|this, _, window, cx| this.jump(false, window, cx))),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(palette.muted)
                    .px_2()
                    .child(format!("{count} hunk{}", if count == 1 { "" } else { "s" })),
            )
            .child(
                Button::new("all")
                    .disabled(self.busy)
                    .label("Include all")
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(i) = this.session.active {
                            this.session.files[i].select(true);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("none")
                    .disabled(self.busy)
                    .label("Exclude all")
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(i) = this.session.active {
                            this.session.files[i].select(false);
                        }
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("split-view")
                    .label("Side by side")
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.unified = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("unified-view")
                    .label("Unified review")
                    .xsmall()
                    .ghost()
                    .tooltip("Unified review is read-only; use side-by-side to edit")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.unified = true;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("wrap")
                    .label("Soft wrap")
                    .xsmall()
                    .checked(self.wrap)
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        this.wrap = *checked;
                        this.old
                            .update(cx, |s, cx| s.set_soft_wrap(*checked, window, cx));
                        this.working
                            .update(cx, |s, cx| s.set_soft_wrap(*checked, window, cx));
                        this.persist();
                        cx.notify();
                    })),
            )
    }
}
impl Render for DiffApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::new(self.dark);
        cx.on_next_frame(window, |this, _, cx| {
            this.sync_scroll(cx);
            this.update_gutter(cx);
        });
        let repo = self
            .session
            .repo
            .root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let title = self
            .session
            .file()
            .map(|f| f.path.display().to_string())
            .unwrap_or_else(|| "No changes".into());
        let count = self.session.files.len();
        let selected = self.session.files.iter().filter(|f| f.included()).count();
        let unversioned = self.session.file().is_some_and(|f| !f.tracked);
        let header = div()
            .flex()
            .items_center()
            .h(px(50.))
            .flex_shrink_0()
            .px_4()
            .gap_3()
            .bg(palette.panel)
            .border_b_1()
            .border_color(palette.border)
            .child(
                div()
                    .text_size(px(22.))
                    .text_color(rgb(if self.dark { 0x87ADFF } else { 0x315ACC }))
                    .child("difly"),
            )
            .child(div().w(px(1.)).h(px(22.)).bg(palette.border))
            .child(
                Button::new("open-repository")
                    .label("Open repository…")
                    .ghost()
                    .xsmall()
                    .disabled(self.busy || self.save_failed)
                    .on_click(cx.listener(|this, _, window, cx| this.open_repository(window, cx))),
            )
            .child(
                Button::new("undo-revert")
                    .label("Undo revert")
                    .ghost()
                    .xsmall()
                    .disabled(self.busy || self.save_failed || !self.session.can_undo_revert())
                    .on_click(cx.listener(|this, _, window, cx| this.undo_revert(window, cx))),
            )
            .child(repo)
            .child(
                div()
                    .text_color(palette.muted)
                    .child(format!("branch: {}", self.branch)),
            )
            .child(div().flex_1())
            .child(
                Button::new("refresh")
                    .label(if self.save_failed {
                        "Reload from disk…"
                    } else {
                        "Refresh"
                    })
                    .xsmall()
                    .ghost()
                    .disabled(self.busy || self.dialog_open)
                    .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
            )
            .child(
                Button::new("theme")
                    .label(if self.dark {
                        "Light theme"
                    } else {
                        "Dark theme"
                    })
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.dark = !this.dark;
                        Theme::change(
                            if this.dark {
                                ThemeMode::Dark
                            } else {
                                ThemeMode::Light
                            },
                            Some(window),
                            cx,
                        );
                        Theme::global_mut(cx).mono_font_family = "Comic Mono".into();
                        this.refresh_indentation(cx);
                        this.persist();
                        cx.notify();
                    })),
            );
        let sidebar = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.panel)
            .border_r_1()
            .border_color(palette.border)
            .child(
                div()
                    .px_3()
                    .pt_4()
                    .pb_2()
                    .text_size(px(11.))
                    .text_color(palette.muted)
                    .child(format!("COMMIT   {count} files")),
            )
            .child(
                div().px_3().pb_2().child(
                    gpui_kit::component::input::Input::new(&self.filter)
                        .small()
                        .disabled(self.busy),
                ),
            )
            .child(
                div().px_3().pb_2().child(
                    Checkbox::new("group-folders")
                        .label("Group by folder")
                        .xsmall()
                        .checked(self.settings.group_folders)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, grouped: &bool, _, cx| {
                            this.settings.group_folders = *grouped;
                            this.rebuild_tree(cx);
                            this.persist();
                            cx.notify();
                        })),
                ),
            )
            .child(div().flex_1().min_h_0().child(self.file_tree(palette, cx)))
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(palette.border)
                    .text_size(px(11.))
                    .text_color(palette.muted)
                    .child(format!(
                        "{selected} {} included · Space / arrows to navigate",
                        if selected == 1 { "file" } else { "files" }
                    )),
            )
            .child(self.commit_panel(palette, cx));
        let caption = |text: &'static str| {
            div()
                .flex()
                .items_center()
                .h(px(25.))
                .flex_shrink_0()
                .px_3()
                .text_size(px(10.))
                .text_color(palette.muted)
                .child(text)
        };
        let left = div()
            .size_full()
            .flex()
            .flex_col()
            .child(caption("HEAD · read-only"))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().flex_1().min_w_0().child(self.pane(true, palette, cx)))
                    .child(self.gutter(palette, cx)),
            );
        let right = div()
            .size_full()
            .flex()
            .flex_col()
            .child(caption(
                if self.session.file().is_some_and(|f| !f.binary && f.exists) {
                    "WORKING FILE · editable"
                } else {
                    "WORKING FILE · read-only"
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.pane(false, palette, cx)),
            );
        let panes = h_resizable("diff-panes")
            .child(resizable_panel().child(left))
            .child(resizable_panel().child(right));
        let pathbar = div()
            .flex()
            .items_center()
            .h(px(37.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(palette.border)
            .child(title)
            .child(div().flex_1())
            .child(
                Button::new("revert-file")
                    .label("Revert file…")
                    .ghost()
                    .xsmall()
                    .disabled(self.busy || self.save_failed || self.session.file().is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.revert_file(window, cx))),
            )
            .when(unversioned, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(0xA36A20))
                        .child("Unversioned"),
                )
            });
        let display = if self.unified {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.review_controls(cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(Diff::new(&self.review).soft_wrap(self.wrap)),
                )
                .into_any_element()
        } else {
            panes.into_any_element()
        };
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .child(pathbar)
            .child(self.toolbar(palette, cx))
            .child(self.conflict_bar(cx))
            .child(div().flex_1().min_h_0().child(display));
        let workspace = h_resizable("workspace")
            .child(resizable_panel().size(px(260.)).child(sidebar))
            .child(resizable_panel().child(content));
        let statusbar = div()
            .flex()
            .items_center()
            .h(px(25.))
            .flex_shrink_0()
            .px_3()
            .bg(palette.panel)
            .border_t_1()
            .border_color(palette.border)
            .text_size(px(10.))
            .text_color(palette.muted)
            .child(self.status.clone())
            .child(div().flex_1())
            .child("F7 / Shift+F7  navigate diffs     ·     Edits autosave");
        div()
            .key_context("DiflyDiff")
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(13.))
            .font_weight(FontWeight::NORMAL)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &ToggleTreeInclusion, _, cx| {
                let id = this
                    .tree
                    .read(cx)
                    .selected_item()
                    .map(|item| item.id.to_string());
                if !this.busy
                    && let Some(indices) = id.and_then(|id| this.tree_members.get(&id))
                {
                    let checked = file_tree::selection_state(&this.session.files, indices)
                        != gpui_kit::base::CheckboxState::Checked;
                    for &i in indices {
                        this.session.files[i].select(checked);
                    }
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Quit, window, cx| {
                if this.guard_close(window, cx) {
                    cx.quit();
                }
            }))
            .on_action(cx.listener(|this, _: &Commit, window, cx| this.commit(window, cx)))
            .on_action(
                cx.listener(|this, _: &OpenRepository, window, cx| {
                    this.open_repository(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &NextHunk, window, cx| this.jump(false, window, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousHunk, window, cx| this.jump(true, window, cx)),
            )
            .child(header)
            .child(div().flex_1().min_h_0().child(workspace))
            .child(statusbar)
    }
}
