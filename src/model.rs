use similar::{ChangeTag, TextDiff};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub selected: bool,
    pub review: bool,
    pub removed: String,
    pub added: String,
}

pub fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// Contiguous edit runs are the selectable unit. Unchanged lines separate hunks.
pub fn hunks(old: &str, new: &str) -> Vec<Hunk> {
    let diff = TextDiff::from_lines(old, new);
    let mut result = Vec::new();
    let (mut a, mut b) = (0, 0);
    let mut pending: Option<Hunk> = None;
    for change in diff.iter_all_changes() {
        if change.tag() == ChangeTag::Equal {
            if let Some(h) = pending.take() {
                result.push(h);
            }
            a += 1;
            b += 1;
        } else {
            let h = pending.get_or_insert_with(|| Hunk {
                old: a..a,
                new: b..b,
                selected: true,
                review: false,
                removed: String::new(),
                added: String::new(),
            });
            match change.tag() {
                ChangeTag::Delete => {
                    a += 1;
                    h.old.end = a;
                    h.removed.push_str(change.value());
                }
                ChangeTag::Insert => {
                    b += 1;
                    h.new.end = b;
                    h.added.push_str(change.value());
                }
                _ => unreachable!(),
            }
        }
    }
    if let Some(h) = pending {
        result.push(h);
    }
    result
}

pub fn refresh_hunks(old: &str, new: &str, previous: &[Hunk], default_selected: bool) -> Vec<Hunk> {
    let mut next = hunks(old, new);
    for h in &mut next {
        // HEAD coordinates stay stable when edits shift working-file line numbers.
        // Match by that location, including zero-width insertion anchors.
        let related: Vec<_> = previous
            .iter()
            .filter(|p| {
                if p.old.is_empty() {
                    h.old.start <= p.old.start && p.old.start <= h.old.end
                } else if h.old.is_empty() {
                    p.old.start <= h.old.start && h.old.start <= p.old.end
                } else {
                    p.old.start < h.old.end && h.old.start < p.old.end
                }
            })
            .collect();
        if let Some(first) = related.first()
            && related
                .iter()
                .all(|p| p.selected == first.selected && p.review == first.review)
        {
            h.selected = first.selected;
            h.review = first.review;
        } else if previous.is_empty() {
            h.selected = default_selected;
        } else {
            // Newly unrelated changes or merges with conflicting inclusion choices
            // still need an explicit decision.
            h.selected = false;
            h.review = true;
        }
    }
    next
}

pub fn selected_text(old: &str, hunks: &[Hunk]) -> String {
    let old_lines = lines(old);
    let mut result = String::new();
    let mut cursor = 0;
    for h in hunks {
        for line in &old_lines[cursor..h.old.start] {
            result.push_str(line);
        }
        if h.selected {
            result.push_str(&h.added);
        } else {
            for line in &old_lines[h.old.clone()] {
                result.push_str(line);
            }
        }
        cursor = h.old.end;
    }
    for line in &old_lines[cursor..] {
        result.push_str(line);
    }
    result
}

pub fn revert_hunk(current: &str, hunk: &Hunk) -> String {
    let mut ls = lines(current);
    ls.splice(hunk.new.clone(), std::iter::once(hunk.removed.as_str()));
    ls.concat()
}

#[derive(Clone, Debug)]
pub struct ConflictBlock {
    pub range: Range<usize>,
    pub ours: String,
    pub theirs: String,
}
pub fn conflicts(text: &str) -> Vec<ConflictBlock> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut start = None;
    let (mut ours, mut theirs) = (String::new(), String::new());
    let mut section = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("<<<<<<< ") {
            start = Some(offset);
            ours.clear();
            theirs.clear();
            section = 1;
        } else if start.is_some() && line.starts_with("||||||| ") {
            section = 0;
        } else if start.is_some() && line.trim_end() == "=======" {
            section = 2;
        } else if let Some(begin) = start.filter(|_| line.starts_with(">>>>>>> ")) {
            out.push(ConflictBlock {
                range: begin..offset + line.len(),
                ours: ours.clone(),
                theirs: theirs.clone(),
            });
            start = None;
            section = 0;
        } else if start.is_some() {
            if section == 1 {
                ours.push_str(line);
            } else if section == 2 {
                theirs.push_str(line);
            }
        }
        offset += line.len();
    }
    out
}

#[derive(Clone, Debug)]
pub struct DiffRows {
    pub old_y: Vec<usize>,
    pub new_y: Vec<usize>,
    pub rows: Vec<(Option<usize>, Option<usize>)>,
    pub hunk_rows: Vec<Range<usize>>,
}
pub fn diff_rows(old: &str, new: &str, hs: &[Hunk], unified: bool) -> DiffRows {
    let old_count = lines(old).len();
    let new_count = lines(new).len();
    let mut result = DiffRows {
        old_y: vec![0; old_count + 1],
        new_y: vec![0; new_count + 1],
        rows: Vec::new(),
        hunk_rows: Vec::new(),
    };
    let (mut a, mut b) = (0, 0);
    let push = |result: &mut DiffRows, x: Option<usize>, y: Option<usize>| {
        let row = result.rows.len();
        if let Some(x) = x {
            result.old_y[x] = row;
        }
        if let Some(y) = y {
            result.new_y[y] = row;
        }
        result.rows.push((x, y));
    };
    for h in hs {
        while a < h.old.start {
            push(&mut result, Some(a), Some(b));
            a += 1;
            b += 1;
        }
        let start = result.rows.len();
        if unified {
            for x in h.old.clone() {
                push(&mut result, Some(x), None);
            }
            for y in h.new.clone() {
                push(&mut result, None, Some(y));
            }
        } else {
            for offset in 0..h.old.len().max(h.new.len()) {
                push(
                    &mut result,
                    (offset < h.old.len()).then_some(h.old.start + offset),
                    (offset < h.new.len()).then_some(h.new.start + offset),
                );
            }
        }
        result.hunk_rows.push(start..result.rows.len());
        a = h.old.end;
        b = h.new.end;
    }
    while a < old_count || b < new_count {
        push(
            &mut result,
            (a < old_count).then_some(a),
            (b < new_count).then_some(b),
        );
        a += 1;
        b += 1;
    }
    // Include the final cursor row for newline-terminated and empty documents.
    let end = result.rows.len();
    *result.old_y.last_mut().unwrap() = end;
    *result.new_y.last_mut().unwrap() = end;
    result
}
