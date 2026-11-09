use difly::git::File;
use gpui_kit::component::tree::TreeItem;
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    files: Vec<(usize, String)>,
}
fn emit(folder: Folder, prefix: &str, members: &mut HashMap<String, Vec<usize>>) -> Vec<TreeItem> {
    let mut result = vec![];
    for (name, child) in folder.folders {
        let id = format!("{prefix}/{name}");
        let children = emit(child, &id, members);
        let indices = children
            .iter()
            .flat_map(|c| members[&c.id.to_string()].clone())
            .collect();
        members.insert(id.clone(), indices);
        result.push(TreeItem::new(id, name).children(children).expanded(true));
    }
    for (index, name) in folder.files {
        let id = format!("file:{index}");
        members.insert(id.clone(), vec![index]);
        result.push(TreeItem::new(id, name));
    }
    result
}
pub fn build(
    files: &[File],
    grouped: bool,
    filter: &str,
) -> (Vec<TreeItem>, HashMap<String, Vec<usize>>) {
    let filter = filter.to_lowercase();
    let mut roots = vec![];
    let mut members = HashMap::new();
    for (tracked, key, label) in [
        (true, "changes", "Changes"),
        (false, "unversioned", "Unversioned files"),
    ] {
        let mut folder = Folder::default();
        for (i, file) in files.iter().enumerate().filter(|(_, f)| {
            f.tracked == tracked && f.path.to_string_lossy().to_lowercase().contains(&filter)
        }) {
            let parts: Vec<_> = file
                .path
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            if !grouped {
                folder
                    .files
                    .push((i, file.path.to_string_lossy().into_owned()));
                continue;
            }
            let mut node = &mut folder;
            for part in &parts[..parts.len().saturating_sub(1)] {
                node = node.folders.entry(part.clone()).or_default();
            }
            node.files
                .push((i, parts.last().cloned().unwrap_or_default()));
        }
        let children = emit(folder, key, &mut members);
        if children.is_empty() {
            continue;
        }
        let indices: Vec<_> = children
            .iter()
            .flat_map(|c| members[&c.id.to_string()].clone())
            .collect();
        let count = indices.len();
        members.insert(key.into(), indices);
        roots.push(
            TreeItem::new(key, format!("{label}  {count}"))
                .children(children)
                .expanded(true),
        );
    }
    (roots, members)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folders_aggregate_descendants_and_duplicate_names_keep_distinct_ids() {
        let mut root = Folder::default();
        root.folders
            .entry("src".into())
            .or_default()
            .files
            .push((0, "index.rs".into()));
        root.folders
            .entry("tests".into())
            .or_default()
            .files
            .push((1, "index.rs".into()));
        let mut members = HashMap::new();
        let nodes = emit(root, "changes", &mut members);
        assert_eq!(nodes.len(), 2);
        assert_eq!(members["changes/src"], vec![0]);
        assert_eq!(members["changes/tests"], vec![1]);
        assert_eq!(nodes[0].children[0].id.as_str(), "file:0");
        assert_eq!(nodes[1].children[0].id.as_str(), "file:1");
    }
}

/// A partial file is mixed even when its whole-file flag is false.
pub fn selection_state(files: &[File], indices: &[usize]) -> gpui_kit::base::CheckboxState {
    use gpui_kit::base::CheckboxState::*;
    let mut any = false;
    let mut all = !indices.is_empty();
    for &index in indices {
        let file = &files[index];
        let (some, every) = if file.hunks.is_empty() {
            (file.selected, file.selected)
        } else {
            (
                file.hunks.iter().any(|h| h.selected),
                file.hunks.iter().all(|h| h.selected),
            )
        };
        any |= some;
        all &= every;
    }
    if all {
        Checked
    } else if any {
        Indeterminate
    } else {
        Unchecked
    }
}

#[cfg(test)]
mod selection_tests {
    use super::selection_state;
    use difly::{git::File, model};
    use gpui_kit::base::CheckboxState::{Checked, Indeterminate, Unchecked};
    #[test]
    fn file_and_folder_states_distinguish_partial_inclusion() {
        let mut file = File {
            path: "code.rs".into(),
            old: vec![],
            current: vec![],
            exists: true,
            base_exists: true,
            tracked: true,
            conflict: false,
            binary: false,
            mode: "100644".into(),
            old_mode: "100644".into(),
            hunks: model::hunks("a\nkeep\nb\n", "A\nkeep\nB\n"),
            selected: false,
            loaded: true,
            source: None,
        };
        assert_eq!(selection_state(&[file.clone()], &[0]), Checked);
        file.hunks[0].selected = false;
        assert_eq!(selection_state(&[file.clone()], &[0]), Indeterminate);
        let mut excluded = file.clone();
        excluded.select(false);
        assert_eq!(selection_state(&[excluded.clone()], &[0]), Unchecked);
        assert_eq!(super::status_label(&file), None);
        file.base_exists = false;
        assert_eq!(super::status_label(&file), Some("New"));
        file.exists = false;
        assert_eq!(super::status_label(&file), Some("Deleted"));
        file.conflict = true;
        assert_eq!(super::status_label(&file), Some("Conflict"));
        assert_eq!(selection_state(&[file, excluded], &[0, 1]), Indeterminate);
        assert_eq!(selection_state(&[], &[]), Unchecked);
    }
}

/// Labels describe existence relative to HEAD, independent of the ignored staging split.
pub fn status_label(file: &File) -> Option<&'static str> {
    if file.conflict {
        Some("Conflict")
    } else if !file.exists {
        Some("Deleted")
    } else if !file.base_exists {
        Some("New")
    } else {
        None
    }
}
