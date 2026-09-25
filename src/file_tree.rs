use difly::git::File;
use eframe::egui::{self, RichText};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Node {
    path: PathBuf,
    tracked: bool,
    folder: bool,
}
#[derive(Clone)]
struct Row {
    node: Node,
    parent: Option<Node>,
}
pub struct FileTree {
    pub grouped: bool,
    visible: Vec<Row>,
    focused: Option<Node>,
    focused_row: Option<egui::Id>,
    pending_focus: Option<Node>,
    closed: HashSet<Node>,
    row_ids: HashMap<Node, egui::Id>,
}
impl Default for FileTree {
    fn default() -> Self {
        Self {
            grouped: true,
            visible: vec![],
            focused: None,
            focused_row: None,
            pending_focus: None,
            closed: HashSet::new(),
            row_ids: HashMap::new(),
        }
    }
}
#[derive(Default)]
pub struct TreeAction {
    pub chosen: Option<PathBuf>,
    pub selection_changed: bool,
}
impl FileTree {
    pub fn navigate(&mut self, ctx: &egui::Context, files: &mut [File]) -> TreeAction {
        let mut action = TreeAction::default();
        if self.focused_row.is_none() || ctx.memory(|m| m.focused()) != self.focused_row {
            return action;
        }
        let key = ctx.input_mut(|i| {
            [
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::Home,
                egui::Key::End,
                egui::Key::Space,
            ]
            .into_iter()
            .find(|&k| i.consume_key(egui::Modifiers::NONE, k))
        });
        let Some(key) = key else {
            return action;
        };
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        let Some(index) = self
            .visible
            .iter()
            .position(|r| Some(&r.node) == self.focused.as_ref())
        else {
            return action;
        };
        let row = self.visible[index].clone();
        let node = &row.node;
        let target = match key {
            egui::Key::Space => {
                let indices = members(files, node);
                let (all, _) = inclusion_state(files, &indices);
                for i in indices {
                    files[i].select(!all);
                    action.selection_changed = true;
                }
                None
            }
            egui::Key::ArrowLeft if node.folder && !self.closed.contains(node) => {
                self.closed.insert(node.clone());
                None
            }
            egui::Key::ArrowLeft => row.parent,
            egui::Key::ArrowRight if node.folder && self.closed.remove(node) => None,
            egui::Key::ArrowRight | egui::Key::ArrowDown => Some(
                self.visible[(index + 1).min(self.visible.len() - 1)]
                    .node
                    .clone(),
            ),
            egui::Key::ArrowUp => Some(self.visible[index.saturating_sub(1)].node.clone()),
            egui::Key::Home => self.visible.first().map(|r| r.node.clone()),
            egui::Key::End => self.visible.last().map(|r| r.node.clone()),
            _ => None,
        };
        if let Some(target) = target {
            if !target.folder {
                action.chosen = Some(target.path.clone());
            }
            self.pending_focus = Some(target);
        }
        action
    }
    pub fn begin_frame(&mut self) {
        self.visible.clear();
        self.row_ids.clear();
    }

    pub fn focus_area(&mut self, ctx: &egui::Context, active: Option<&PathBuf>) {
        let node = self
            .visible
            .iter()
            .find(|r| Some(&r.node) == self.focused.as_ref())
            .or_else(|| {
                self.visible
                    .iter()
                    .find(|r| !r.node.folder && Some(&r.node.path) == active)
            })
            .or_else(|| self.visible.first())
            .map(|r| r.node.clone());
        if let Some(node) = node
            && let Some(&id) = self.row_ids.get(&node)
        {
            ctx.memory_mut(|m| m.request_focus(id));
            self.focused = Some(node);
            self.focused_row = Some(id);
        }
    }

    fn row_selected(&self, node: &Node, active: Option<&PathBuf>) -> bool {
        // The editor's active file may remain open while navigating folders.
        // Only the tree cursor owns the selection highlight.
        match self.pending_focus.as_ref().or(self.focused.as_ref()) {
            Some(cursor) => cursor == node,
            None => !node.folder && Some(&node.path) == active,
        }
    }

    fn focus_row(&mut self, ui: &egui::Ui, row: &egui::Response, node: &Node, clicked: bool) {
        self.row_ids.insert(node.clone(), row.id);
        if clicked || self.pending_focus.as_ref() == Some(node) {
            row.request_focus();
            if !clicked {
                row.scroll_to_me(Some(egui::Align::Center));
            }
            self.pending_focus = None;
        }
        if row.has_focus() {
            self.focused = Some(node.clone());
            self.focused_row = Some(row.id);
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    row.id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        horizontal_arrows: true,
                        ..Default::default()
                    },
                )
            });
        }
    }

    pub fn show_group(
        &mut self,
        ui: &mut egui::Ui,
        files: &mut [File],
        indices: &[usize],
        label: &str,
        active: Option<&PathBuf>,
    ) -> TreeAction {
        let Some(&first) = indices.first() else {
            return TreeAction::default();
        };
        let node = Node {
            path: PathBuf::new(),
            tracked: files[first].tracked,
            folder: true,
        };
        self.visible.push(Row {
            node: node.clone(),
            parent: None,
        });
        ui.push_id(&node, |ui| {
            let mut action = TreeAction {
                selection_changed: self.folder_header(ui, files, &node, label),
                ..Default::default()
            };
            if !self.closed.contains(&node) {
                let child = ui
                    .indent("children", |ui| {
                        self.show_children(ui, files, indices, 0, active, Some(node.clone()))
                    })
                    .inner;
                action.chosen = child.chosen;
                action.selection_changed |= child.selection_changed;
            }
            action
        })
        .inner
    }

    fn folder_header(
        &mut self,
        ui: &mut egui::Ui,
        files: &mut [File],
        node: &Node,
        label: &str,
    ) -> bool {
        let indices = members(files, node);
        let (mut checked, partial) = inclusion_state(files, &indices);
        let mut changed = false;
        ui.horizontal(|ui| {
            let open = !self.closed.contains(node);
            let arrow = disclosure(ui, open);
            let check = ui
                .add_enabled_ui(!indices.is_empty(), |ui| {
                    inclusion_checkbox(ui, &mut checked, partial)
                })
                .inner;
            if check.changed() {
                for &i in &indices {
                    files[i].select(checked);
                }
                changed = true;
            }
            let text = if node.path.as_os_str().is_empty() {
                RichText::new(label).strong()
            } else {
                RichText::new(label)
            };
            let row = ui.selectable_label(self.row_selected(node, None), text);
            ui.weak(format!("{} files", indices.len()));
            if arrow.clicked() || row.double_clicked() {
                if open {
                    self.closed.insert(node.clone());
                } else {
                    self.closed.remove(node);
                }
            }
            self.focus_row(
                ui,
                &row,
                node,
                row.clicked() || check.clicked() || arrow.clicked(),
            );
        });
        changed
    }

    #[cfg(test)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        files: &mut [File],
        indices: &[usize],
        depth: usize,
        active: Option<&PathBuf>,
    ) -> TreeAction {
        self.show_children(ui, files, indices, depth, active, None)
    }
    fn show_children(
        &mut self,
        ui: &mut egui::Ui,
        files: &mut [File],
        indices: &[usize],
        depth: usize,
        active: Option<&PathBuf>,
        parent: Option<Node>,
    ) -> TreeAction {
        let mut action = TreeAction::default();
        let mut directories: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &i in indices {
            let components: Vec<_> = files[i].path.components().collect();
            if self.grouped && components.len() > depth + 1 {
                directories
                    .entry(components[depth].as_os_str().to_string_lossy().into())
                    .or_default()
                    .push(i);
                continue;
            }
            let file = &mut files[i];
            let node = Node {
                path: file.path.clone(),
                tracked: file.tracked,
                folder: false,
            };
            self.visible.push(Row {
                node: node.clone(),
                parent: parent.clone(),
            });
            ui.push_id(&node, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    let mut checked = file.included();
                    let partial = checked && file.hunks.iter().any(|h| !h.selected);
                    let check = ui
                        .add_enabled_ui(!file.conflict, |ui| {
                            inclusion_checkbox(ui, &mut checked, partial)
                        })
                        .inner;
                    if check.changed() {
                        file.select(checked);
                        action.selection_changed = true;
                    }
                    let label = if self.grouped {
                        file.path.file_name().unwrap_or_default().to_string_lossy()
                    } else {
                        file.path.to_string_lossy()
                    };
                    let color = if file.conflict || !file.tracked {
                        ui.visuals().warn_fg_color
                    } else if !file.exists {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().hyperlink_color
                    };
                    let row = ui
                        .selectable_label(
                            self.row_selected(&node, active),
                            RichText::new(label).color(color),
                        )
                        .on_hover_text(file.path.display().to_string());
                    if row.clicked() || check.clicked() {
                        action.chosen = Some(file.path.clone());
                    }
                    self.focus_row(ui, &row, &node, row.clicked() || check.clicked());
                    if partial {
                        ui.weak("partial");
                    }
                });
            });
        }
        for (name, children) in directories {
            let path = files[children[0]]
                .path
                .components()
                .take(depth + 1)
                .collect::<PathBuf>();
            let node = Node {
                path,
                tracked: files[children[0]].tracked,
                folder: true,
            };
            self.visible.push(Row {
                node: node.clone(),
                parent: parent.clone(),
            });
            ui.push_id(&node, |ui| {
                action.selection_changed |=
                    self.folder_header(ui, files, &node, &format!("{name}/"));
                if !self.closed.contains(&node) {
                    let child = ui
                        .indent("children", |ui| {
                            self.show_children(
                                ui,
                                files,
                                &children,
                                depth + 1,
                                active,
                                Some(node.clone()),
                            )
                        })
                        .inner;
                    if child.chosen.is_some() {
                        action.chosen = child.chosen;
                    }
                    action.selection_changed |= child.selection_changed;
                }
            });
        }
        action
    }
}

fn members(files: &[File], node: &Node) -> Vec<usize> {
    files
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.tracked == node.tracked
                && !f.conflict
                && if node.folder {
                    node.path.as_os_str().is_empty() || f.path.starts_with(&node.path)
                } else {
                    f.path == node.path
                }
        })
        .map(|(i, _)| i)
        .collect()
}

fn inclusion_state(files: &[File], indices: &[usize]) -> (bool, bool) {
    let any = indices.iter().any(|&i| files[i].included());
    let all = !indices.is_empty()
        && indices
            .iter()
            .all(|&i| files[i].included() && files[i].hunks.iter().all(|h| h.selected));
    (all, any && !all)
}

fn disclosure(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let sense = egui::Sense::click().difference(egui::Sense::focusable_noninteractive());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(10.0, ui.spacing().interact_size.y), sense);
    let center = rect.center();
    let points = if open {
        vec![
            center + egui::vec2(-3.0, -2.0),
            center + egui::vec2(0.0, 1.0),
            center + egui::vec2(3.0, -2.0),
        ]
    } else {
        vec![
            center + egui::vec2(-1.0, -3.0),
            center + egui::vec2(2.0, 0.0),
            center + egui::vec2(-1.0, 3.0),
        ]
    };
    ui.painter().add(egui::Shape::line(
        points,
        ui.style().interact(&response).fg_stroke,
    ));
    response
}

/// Mouse-clickable checkbox with no independent keyboard focus stop.
pub fn inclusion_checkbox(ui: &mut egui::Ui, checked: &mut bool, partial: bool) -> egui::Response {
    let sense = egui::Sense::click().difference(egui::Sense::focusable_noninteractive());
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(20.0, ui.spacing().interact_size.y), sense);
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled(),
            *checked,
            "Include change",
        )
    });
    let visuals = ui.style().interact(&response);
    let icon = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(15.0));
    ui.painter().rect(
        icon,
        2.0,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    if partial {
        ui.painter().hline(
            icon.shrink(3.0).x_range(),
            icon.center().y,
            visuals.fg_stroke,
        );
    } else if *checked {
        ui.painter().add(egui::Shape::line(
            vec![
                icon.left_center() + egui::vec2(3.0, 0.0),
                icon.center_bottom() + egui::vec2(-1.0, -3.0),
                icon.right_top() + egui::vec2(-2.0, 3.0),
            ],
            visuals.fg_stroke,
        ));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(path: &str) -> File {
        File {
            path: path.into(),
            old: vec![],
            current: vec![],
            exists: true,
            base_exists: true,
            tracked: true,
            conflict: false,
            binary: false,
            mode: "100644".into(),
            old_mode: "100644".into(),
            hunks: vec![],
            selected: true,
        }
    }
    struct Harness {
        ctx: egui::Context,
        tree: FileTree,
        files: Vec<File>,
        active: PathBuf,
    }
    impl Harness {
        fn new() -> Self {
            let mut h = Self {
                ctx: Default::default(),
                tree: Default::default(),
                files: vec![
                    file("a.rs"),
                    file("folder/b.rs"),
                    file("folder/nested/c.rs"),
                ],
                active: "a.rs".into(),
            };
            h.tree.pending_focus = Some(Node {
                path: "a.rs".into(),
                tracked: true,
                folder: false,
            });
            h.frame(None);
            h
        }
        fn frame(&mut self, key: Option<egui::Key>) -> TreeAction {
            let input = egui::RawInput {
                events: key
                    .map(|key| {
                        vec![egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }]
                    })
                    .unwrap_or_default(),
                ..Default::default()
            };
            let mut action = TreeAction::default();
            let _ = self.ctx.run(input, |ctx| {
                action = self.tree.navigate(ctx, &mut self.files);
                if let Some(path) = &action.chosen {
                    self.active = path.clone();
                }
                self.tree.begin_frame();
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.tree
                        .show(ui, &mut self.files, &[0, 1, 2], 0, Some(&self.active));
                });
            });
            action
        }
        fn focused(&self) -> &str {
            self.tree.focused.as_ref().unwrap().path.to_str().unwrap()
        }
    }
    #[test]
    fn arrows_follow_tree_parent_and_expansion_rules() {
        use egui::Key::*;
        let mut h = Harness::new();
        h.frame(Some(ArrowDown));
        assert_eq!(h.focused(), "folder");
        h.frame(Some(ArrowRight));
        assert_eq!(h.focused(), "folder/b.rs");
        h.frame(Some(ArrowLeft));
        assert_eq!(h.focused(), "folder");
        h.frame(Some(ArrowLeft));
        assert_eq!(h.tree.visible.len(), 2);
        h.frame(Some(ArrowLeft));
        assert_eq!(h.focused(), "folder");
        h.frame(Some(ArrowRight));
        assert_eq!(h.focused(), "folder");
        assert_eq!(h.tree.visible.len(), 5);
        h.frame(Some(ArrowRight));
        assert_eq!(h.focused(), "folder/b.rs");
        h.frame(Some(ArrowRight));
        assert_eq!(h.focused(), "folder/nested");
        h.frame(Some(ArrowLeft));
        assert_eq!(h.tree.visible.len(), 4);
        h.frame(Some(ArrowLeft));
        assert_eq!(h.focused(), "folder");
        h.frame(Some(End));
        assert_eq!(h.focused(), "folder/nested");
        h.frame(Some(ArrowRight));
        assert_eq!(h.focused(), "folder/nested");
        h.frame(Some(ArrowRight));
        assert_eq!(h.active, PathBuf::from("folder/nested/c.rs"));
        h.frame(Some(ArrowRight));
        assert_eq!(h.focused(), "folder/nested/c.rs");
        h.frame(Some(Home));
        assert_eq!(h.focused(), "a.rs");
        h.frame(Some(ArrowUp));
        assert_eq!(h.focused(), "a.rs");
    }
    #[test]
    fn space_toggles_without_moving_focus_and_skips_conflicts() {
        use egui::Key::*;
        let mut h = Harness::new();
        assert!(h.frame(Some(Space)).selection_changed);
        assert!(!h.files[0].included());
        assert_eq!(h.focused(), "a.rs");
        h.frame(Some(Space));
        assert!(h.files[0].included());
        h.frame(Some(ArrowDown));
        h.frame(Some(Space));
        assert!(!h.files[1].included());
        assert!(!h.files[2].included());
        h.files[2].conflict = true;
        h.frame(Some(Space));
        assert!(h.files[1].included());
        assert!(!h.files[2].included());
        h.ctx
            .memory_mut(|m| m.request_focus(egui::Id::new("editor")));
        assert!(!h.frame(Some(Space)).selection_changed);
    }
    #[test]
    fn flat_list_navigation_and_tab_skip_checkboxes() {
        use egui::Key::*;
        let mut h = Harness::new();
        h.tree.grouped = false;
        h.frame(None);
        h.frame(Some(Tab));
        h.frame(None);
        assert_eq!(h.focused(), "folder/b.rs");
        h.frame(Some(ArrowRight));
        assert_eq!(h.focused(), "folder/nested/c.rs");
        h.frame(Some(ArrowLeft));
        assert_eq!(h.focused(), "folder/nested/c.rs");
        assert_eq!(h.tree.visible.len(), 3);
    }
    #[test]
    fn root_groups_navigate_collapse_and_toggle_independently() {
        let ctx = egui::Context::default();
        let mut tree = FileTree::default();
        let mut files = vec![file("folder/a.rs"), file("folder/b.rs"), file("new.rs")];
        files[2].tracked = false;
        files[2].select(false);
        let root = Node {
            path: PathBuf::new(),
            tracked: true,
            folder: true,
        };
        tree.pending_focus = Some(root.clone());
        let frame = |tree: &mut FileTree, files: &mut [File], key: Option<egui::Key>| {
            let input = egui::RawInput {
                events: key
                    .map(|key| {
                        vec![egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }]
                    })
                    .unwrap_or_default(),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                tree.navigate(ctx, files);
                tree.begin_frame();
                egui::CentralPanel::default().show(ctx, |ui| {
                    tree.show_group(ui, files, &[0, 1], "Changes", None);
                    tree.show_group(ui, files, &[2], "Unversioned Files", None);
                });
            });
        };
        frame(&mut tree, &mut files, None);
        assert_eq!(tree.visible.len(), 6);
        frame(&mut tree, &mut files, Some(egui::Key::Space));
        assert!(files.iter().all(|f| !f.included()));
        frame(&mut tree, &mut files, Some(egui::Key::Space));
        assert!(files[0].included() && files[1].included() && !files[2].included());
        frame(&mut tree, &mut files, Some(egui::Key::ArrowLeft));
        assert_eq!(tree.visible.len(), 3);
        frame(&mut tree, &mut files, Some(egui::Key::ArrowRight));
        frame(&mut tree, &mut files, Some(egui::Key::ArrowRight));
        assert_eq!(tree.focused.as_ref().unwrap().path, PathBuf::from("folder"));
        frame(&mut tree, &mut files, Some(egui::Key::ArrowLeft));
        frame(&mut tree, &mut files, Some(egui::Key::ArrowLeft));
        assert_eq!(tree.focused, Some(root));
        frame(&mut tree, &mut files, Some(egui::Key::End));
        frame(&mut tree, &mut files, Some(egui::Key::ArrowLeft));
        assert!(!tree.focused.as_ref().unwrap().tracked);
        frame(&mut tree, &mut files, Some(egui::Key::Space));
        assert!(files.iter().all(File::included));
    }

    #[test]
    fn folder_checkbox_reports_mixed_descendant_selection() {
        let mut files = vec![file("src/a"), file("src/nested/b"), file("src-other/c")];
        files[1].select(false);
        let node = Node {
            path: "src".into(),
            tracked: true,
            folder: true,
        };
        let indices = members(&files, &node);
        assert_eq!(indices, vec![0, 1]);
        assert_eq!(inclusion_state(&files, &indices), (false, true));
        files[1].select(true);
        assert_eq!(inclusion_state(&files, &indices), (true, false));
        files[0].select(false);
        files[1].select(false);
        assert_eq!(inclusion_state(&files, &indices), (false, false));
    }

    #[test]
    fn background_focus_restores_navigation_without_changing_selection() {
        let mut h = Harness::new();
        h.frame(Some(egui::Key::ArrowDown));
        assert_eq!(h.focused(), "folder");
        h.ctx
            .memory_mut(|m| m.request_focus(egui::Id::new("editor")));
        h.tree.focus_area(&h.ctx, Some(&h.active));
        assert_eq!(h.ctx.memory(|m| m.focused()), h.tree.focused_row);
        assert_eq!(h.focused(), "folder");
        h.frame(Some(egui::Key::ArrowRight));
        assert_eq!(h.focused(), "folder/b.rs");
        assert!(h.files.iter().all(File::included));
        // With no remembered focus, prefer the active file over the first row.
        h.tree.focused = None;
        h.tree.focus_area(&h.ctx, Some(&h.active));
        assert_eq!(h.focused(), "folder/b.rs");
        h.tree.begin_frame();
        h.tree.focus_area(&h.ctx, None); // An empty/filtered tree is harmless.
    }

    #[test]
    fn tree_cursor_and_active_editor_never_highlight_two_rows() {
        let mut h = Harness::new();
        h.frame(Some(egui::Key::End));
        assert_eq!(h.active, PathBuf::from("folder/nested/c.rs"));
        h.frame(Some(egui::Key::ArrowLeft));
        assert_eq!(h.focused(), "folder/nested");
        let selected: Vec<_> = h
            .tree
            .visible
            .iter()
            .filter(|r| h.tree.row_selected(&r.node, Some(&h.active)))
            .collect();
        assert_eq!(selected.len(), 1);
        assert!(selected[0].node.folder);
        h.frame(Some(egui::Key::ArrowLeft)); // collapse, leaving active file hidden
        assert_eq!(
            h.tree
                .visible
                .iter()
                .filter(|r| h.tree.row_selected(&r.node, Some(&h.active)))
                .count(),
            1
        );
        h.frame(Some(egui::Key::Home));
        assert_eq!(
            h.tree
                .visible
                .iter()
                .filter(|r| h.tree.row_selected(&r.node, Some(&h.active)))
                .count(),
            1
        );
    }

    #[test]
    fn inclusion_checkbox_is_not_focusable() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = inclusion_checkbox(ui, &mut true, false);
                assert!(!response.sense.is_focusable());
                assert!(response.sense.senses_click());
            });
        });
    }
}
