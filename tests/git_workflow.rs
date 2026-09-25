use difly::{git::Repo, model};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
fn repo() -> (TempDir, Repo) {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-b", "main"]);
    git(temp.path(), &["config", "user.name", "Difly Test"]);
    git(
        temp.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(temp.path(), &["config", "commit.gpgsign", "false"]);
    git(temp.path(), &["config", "core.autocrlf", "false"]);
    let repo = Repo::open(temp.path()).unwrap();
    (temp, repo)
}
fn seed(repo: &Repo) {
    fs::write(repo.root.join("file.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "-m", "initial"]);
}
#[test]
fn selected_hunks_commit_and_normalize_external_staging() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "ONE\ntwo\nthree\nfour\nFIVE\n").unwrap();
    git(&r.root, &["add", "file.txt"]);
    let mut files = r.scan().unwrap();
    assert_eq!(files[0].hunks.len(), 2);
    files[0].hunks[1].selected = false;
    let head = r.head();
    r.commit(&files, "partial", false, head.as_deref()).unwrap();
    assert_eq!(
        r.blob("HEAD:file.txt").unwrap(),
        b"ONE\ntwo\nthree\nfour\nfive\n"
    );
    assert_eq!(
        fs::read(r.root.join("file.txt")).unwrap(),
        b"ONE\ntwo\nthree\nfour\nFIVE\n"
    );
    assert!(git(&r.root, &["diff", "--cached"]).is_empty());
    assert!(!git(&r.root, &["diff"]).is_empty());
}
#[test]
fn untracked_default_and_initial_commit() {
    let (_temp, r) = repo();
    fs::write(r.root.join("new file.txt"), "new\n").unwrap();
    let mut files = r.scan().unwrap();
    assert!(!files[0].included());
    files[0].select(true);
    r.commit(&files, "initial", false, None).unwrap();
    assert!(r.scan().unwrap().is_empty());
}
#[test]
fn autosave_rejects_external_changes() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "edit\n").unwrap();
    let mut f = r.scan().unwrap().remove(0);
    r.save(&mut f, "saved\n").unwrap();
    fs::write(r.root.join("file.txt"), "external\n").unwrap();
    assert!(r.save(&mut f, "overwrite\n").is_err());
    assert_eq!(
        fs::read_to_string(r.root.join("file.txt")).unwrap(),
        "external\n"
    );
}
#[test]
fn deletion_binary_and_message_only_amend() {
    let (_temp, r) = repo();
    seed(&r);
    fs::remove_file(r.root.join("file.txt")).unwrap();
    fs::write(r.root.join("binary"), [0, 1, 2]).unwrap();
    let mut files = r.scan().unwrap();
    for f in &mut files {
        f.select(true);
    }
    r.commit(&files, "replace", false, r.head().as_deref())
        .unwrap();
    assert_eq!(r.blob("HEAD:binary").unwrap(), [0, 1, 2]);
    assert!(r.blob("HEAD:file.txt").is_err());
    r.commit(&[], "renamed message", true, r.head().as_deref())
        .unwrap();
    assert_eq!(r.message(), "renamed message");
    assert_eq!(git(&r.root, &["rev-list", "--count", "HEAD"]), "2");
}
#[test]
fn stale_head_or_file_cannot_commit() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "edit\n").unwrap();
    let files = r.scan().unwrap();
    assert!(r.commit(&files, "bad", false, Some("nope")).is_err());
    fs::write(r.root.join("file.txt"), "changed\n").unwrap();
    assert!(r.commit(&files, "bad", false, r.head().as_deref()).is_err());
}
#[cfg(unix)]
#[test]
fn failed_hook_preserves_index_and_worktree() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "edit\n").unwrap();
    git(&r.root, &["add", "."]);
    let before = fs::read(r.root.join(".git/index")).unwrap();
    let head = r.head();
    let hook = r.root.join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho rejected >&2\nexit 1\n").unwrap();
    fs::set_permissions(hook, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        r.commit(&r.scan().unwrap(), "fail", false, head.as_deref())
            .is_err()
    );
    assert_eq!(r.head(), head);
    assert_eq!(fs::read(r.root.join(".git/index")).unwrap(), before);
    assert_eq!(
        fs::read_to_string(r.root.join("file.txt")).unwrap(),
        "edit\n"
    );
}
#[test]
fn merge_resolution_creates_two_parent_commit() {
    let (_temp, r) = repo();
    seed(&r);
    git(&r.root, &["checkout", "-b", "other"]);
    fs::write(r.root.join("file.txt"), "theirs\n").unwrap();
    git(&r.root, &["commit", "-am", "theirs"]);
    git(&r.root, &["checkout", "main"]);
    fs::write(r.root.join("file.txt"), "ours\n").unwrap();
    git(&r.root, &["commit", "-am", "ours"]);
    let result = Command::new("git")
        .arg("-C")
        .arg(&r.root)
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let mut files = r.scan().unwrap();
    assert!(files[0].conflict);
    assert!(
        r.commit(&files, "merge", false, r.head().as_deref())
            .is_err()
    );
    assert_eq!(r.conflict_stage(&files[0].path, 2).unwrap(), "ours\n");
    assert_eq!(r.conflict_stage(&files[0].path, 3).unwrap(), "theirs\n");
    r.save(&mut files[0], "resolved\n").unwrap();
    r.resolve(&files[0]).unwrap();
    r.commit(&r.scan().unwrap(), "merge", false, r.head().as_deref())
        .unwrap();
    assert_eq!(
        git(&r.root, &["rev-list", "--parents", "-1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
    assert!(!r.merge());
}
#[test]
fn hunk_reconstruction_revert_and_selection_review() {
    let old = "a\nb\nc\nd";
    let new = "A\nb\nc\nD\nextra";
    let mut hs = model::hunks(old, new);
    assert_eq!(model::selected_text(old, &hs), new);
    for h in &mut hs {
        h.selected = false;
    }
    assert_eq!(model::selected_text(old, &hs), old);
    assert_eq!(model::revert_hunk(new, &hs[0]), "a\nb\nc\nD\nextra");
    let next = model::refresh_hunks(old, "AA\nb\nc\nD\nextra", &hs, true);
    assert!(!next[0].review);
    assert!(!next[0].selected);
    assert!(!next[1].review);
}

#[test]
fn empty_files_and_staged_additions_are_not_lost() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("empty"), "").unwrap();
    fs::write(r.root.join("staged"), "staged\n").unwrap();
    git(&r.root, &["add", "staged"]);
    let mut files = r.scan().unwrap();
    assert!(
        files
            .iter()
            .find(|f| f.path == Path::new("staged"))
            .unwrap()
            .included()
    );
    assert!(
        !files
            .iter()
            .find(|f| f.path == Path::new("empty"))
            .unwrap()
            .included()
    );
    for f in &mut files {
        f.select(true);
    }
    r.commit(&files, "add", false, r.head().as_deref()).unwrap();
    assert_eq!(r.blob("HEAD:empty").unwrap(), b"");
    assert_eq!(r.blob("HEAD:staged").unwrap(), b"staged\n");
}

#[test]
fn revert_and_restore_binary_and_deleted_file() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("binary"), [0, 2, 3]).unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "binary"]);
    fs::write(r.root.join("binary"), [0, 9, 8]).unwrap();
    fs::remove_file(r.root.join("file.txt")).unwrap();
    for f in r.scan().unwrap() {
        r.restore_work(&f, Some(&f.old), &f.old_mode).unwrap();
    }
    assert!(r.scan().unwrap().is_empty());
}

#[test]
fn crlf_attributes_do_not_turn_the_entire_file_into_a_hunk() {
    let (_temp, r) = repo();
    fs::write(r.root.join(".gitattributes"), "*.txt text eol=crlf\n").unwrap();
    fs::write(r.root.join("file.txt"), "a\r\nb\r\nc\r\n").unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "crlf"]);
    fs::write(r.root.join("file.txt"), "a\r\nB\r\nc\r\n").unwrap();
    let files = r.scan().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].hunks.len(), 1);
    assert_eq!(files[0].hunks[0].removed, "b\r\n");
    r.commit(&files, "edit", false, r.head().as_deref())
        .unwrap();
    assert_eq!(r.blob("HEAD:file.txt").unwrap(), b"a\nB\nc\n");
    assert_eq!(
        fs::read(r.root.join("file.txt")).unwrap(),
        b"a\r\nB\r\nc\r\n"
    );
    assert!(r.scan().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn symbolic_links_revert_without_touching_target_and_modes_commit() {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    };
    let (_temp, r) = repo();
    seed(&r);
    symlink("file.txt", r.root.join("link")).unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "link"]);
    fs::remove_file(r.root.join("link")).unwrap();
    symlink("absent", r.root.join("link")).unwrap();
    let f = r.scan().unwrap().remove(0);
    assert!(f.binary);
    r.restore_work(&f, Some(&f.old), &f.old_mode).unwrap();
    assert_eq!(
        fs::read_link(r.root.join("link"))
            .unwrap()
            .as_os_str()
            .as_bytes(),
        b"file.txt"
    );
    fs::set_permissions(r.root.join("file.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    r.commit(&r.scan().unwrap(), "executable", false, r.head().as_deref())
        .unwrap();
    assert!(git(&r.root, &["ls-tree", "HEAD", "file.txt"]).starts_with("100755"));
}

#[test]
fn literal_pathspecs_do_not_touch_neighboring_files() {
    let (_temp, r) = repo();
    seed(&r);
    for name in ["[abc].txt", "a.txt", "-option.txt"] {
        fs::write(r.root.join(name), "old\n").unwrap();
    }
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "names"]);
    for name in ["[abc].txt", "a.txt", "-option.txt"] {
        fs::write(r.root.join(name), "new\n").unwrap();
    }
    let mut files = r.scan().unwrap();
    for f in &mut files {
        f.select(f.path != Path::new("a.txt"));
    }
    r.commit(&files, "literal", false, r.head().as_deref())
        .unwrap();
    assert_eq!(r.blob("HEAD:a.txt").unwrap(), b"old\n");
    assert_eq!(r.blob("HEAD:[abc].txt").unwrap(), b"new\n");
    assert_eq!(r.blob("HEAD:-option.txt").unwrap(), b"new\n");
}

#[test]
fn malformed_conflict_markers_cannot_be_marked_resolved() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "<<<<<<< ours\ncontent\n").unwrap();
    let mut file = r.scan().unwrap().remove(0);
    file.conflict = true;
    assert!(r.resolve(&file).is_err());
}

#[test]
fn conflict_choices_and_diff_alignment_preserve_content() {
    let text = "before\n<<<<<<< ours\nleft\n||||||| base\noriginal\n=======\nright\n>>>>>>> theirs\nafter\n";
    let blocks = model::conflicts(text);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].ours, "left\n");
    assert_eq!(blocks[0].theirs, "right\n");
    let mut result = text.to_string();
    result.replace_range(
        blocks[0].range.clone(),
        &format!("{}{}", blocks[0].ours, blocks[0].theirs),
    );
    assert_eq!(result, "before\nleft\nright\nafter\n");
    let samples = [
        "",
        "a",
        "a\n",
        "a\nb\nc\n",
        "x\na\nc\ny",
        "日本語\n🦀\n",
        "a\r\nb\r\n",
    ];
    for old in samples {
        for new in samples {
            let mut hs = model::hunks(old, new);
            assert_eq!(model::selected_text(old, &hs), new);
            for unified in [true, false] {
                let rows = model::diff_rows(old, new, &hs, unified);
                for (row, (a, b)) in rows.rows.iter().enumerate() {
                    if let Some(a) = a {
                        assert_eq!(rows.old_y[*a], row);
                    }
                    if let Some(b) = b {
                        assert_eq!(rows.new_y[*b], row);
                    }
                }
                assert!(rows.old_y.windows(2).all(|w| w[0] < w[1]));
                assert!(rows.new_y.windows(2).all(|w| w[0] < w[1]));
            }
            for h in &mut hs {
                h.selected = false;
            }
            assert_eq!(model::selected_text(old, &hs), old);
        }
    }
}

#[test]
fn modify_delete_merge_can_resolve_to_deletion() {
    let (_temp, r) = repo();
    seed(&r);
    git(&r.root, &["checkout", "-b", "other"]);
    fs::write(r.root.join("file.txt"), "theirs\n").unwrap();
    git(&r.root, &["commit", "-am", "modify"]);
    git(&r.root, &["checkout", "main"]);
    git(&r.root, &["rm", "file.txt"]);
    git(&r.root, &["commit", "-m", "delete"]);
    let result = Command::new("git")
        .arg("-C")
        .arg(&r.root)
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let mut file = r.scan().unwrap().remove(0);
    assert!(file.conflict);
    assert!(r.conflict_blob(&file.path, 2).unwrap().is_none());
    r.restore_work(&file, None, &file.mode).unwrap();
    file.exists = false;
    file.current.clear();
    r.resolve(&file).unwrap();
    r.commit(
        &r.scan().unwrap(),
        "resolve deletion",
        false,
        r.head().as_deref(),
    )
    .unwrap();
    assert!(r.blob("HEAD:file.txt").is_err());
    assert_eq!(
        git(&r.root, &["rev-list", "--parents", "-1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
}

#[test]
fn binary_merge_can_resolve_to_theirs() {
    let (_temp, r) = repo();
    fs::write(r.root.join("binary"), [0, 1]).unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "base"]);
    git(&r.root, &["checkout", "-b", "other"]);
    fs::write(r.root.join("binary"), [0, 2]).unwrap();
    git(&r.root, &["commit", "-am", "theirs"]);
    git(&r.root, &["checkout", "main"]);
    fs::write(r.root.join("binary"), [0, 3]).unwrap();
    git(&r.root, &["commit", "-am", "ours"]);
    let result = Command::new("git")
        .arg("-C")
        .arg(&r.root)
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let mut file = r.scan().unwrap().remove(0);
    assert!(file.binary && file.conflict);
    let theirs = r.conflict_blob(&file.path, 3).unwrap().unwrap();
    r.restore_work(&file, Some(&theirs), &file.mode).unwrap();
    file.current = theirs;
    r.resolve(&file).unwrap();
    r.commit(
        &r.scan().unwrap(),
        "resolve binary",
        false,
        r.head().as_deref(),
    )
    .unwrap();
    assert_eq!(r.blob("HEAD:binary").unwrap(), [0, 2]);
}

#[test]
fn editing_hunks_preserves_inclusion_through_insertions_and_line_shifts() {
    let old = "a\nb\nc\nd\n";
    let mut previous = model::hunks(old, "A\nb\nc\nD\n");
    previous[1].selected = false;
    let next = model::refresh_hunks(old, "AA\nextra\nb\nc\nDD\n", &previous, false);
    assert_eq!(next.len(), 2);
    assert!(next[0].selected && !next[0].review);
    assert!(!next[1].selected && !next[1].review);
    let inserted = model::hunks(old, "a\ninserted\nb\nc\nd\n");
    let next = model::refresh_hunks(old, "a\nchanged\nanother\nb\nc\nd\n", &inserted, false);
    assert!(next[0].selected && !next[0].review);
}

#[test]
fn split_hunks_keep_selection_but_mixed_merges_need_review() {
    let old = "a\nb\nc\n";
    let previous = model::hunks(old, "A\nB\nC\n");
    let split = model::refresh_hunks(old, "A\nb\nC\n", &previous, false);
    assert_eq!(split.len(), 2);
    assert!(split.iter().all(|h| h.selected && !h.review));
    let merged = model::refresh_hunks(old, "A\nB\nC\n", &split, false);
    assert!(merged[0].selected && !merged[0].review);
    let mut mixed = split;
    mixed[1].selected = false;
    let merged = model::refresh_hunks(old, "A\nB\nC\n", &mixed, false);
    assert!(!merged[0].selected && merged[0].review);
}

#[test]
fn autosaving_inside_selected_hunk_keeps_it_ready_to_commit() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "ONE\ntwo\nthree\nfour\nfive\n").unwrap();
    let mut files = r.scan().unwrap();
    r.save(&mut files[0], "ONE edited\ntwo\nthree\nfour\nfive\n")
        .unwrap();
    assert!(files[0].hunks[0].selected && !files[0].hunks[0].review);
    r.commit(&files, "edited hunk", false, r.head().as_deref())
        .unwrap();
    assert_eq!(
        r.blob("HEAD:file.txt").unwrap(),
        b"ONE edited\ntwo\nthree\nfour\nfive\n"
    );
}

#[test]
fn older_amend_moves_selected_hunks_into_target_and_replays_history() {
    let (_temp, r) = repo();
    seed(&r);
    let target = r.head().unwrap();
    fs::write(r.root.join("later.txt"), "later\n").unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "later commit"]);
    let head = r.head().unwrap();
    fs::write(r.root.join("file.txt"), "ONE\ntwo\nthree\nfour\nFIVE\n").unwrap();
    git(&r.root, &["add", "file.txt"]);
    let mut files = r.scan().unwrap();
    files[0].hunks[1].selected = false;
    r.amend_commit(&files, "improved initial", &target, Some(&head))
        .unwrap();
    assert_eq!(
        git(&r.root, &["log", "--format=%s"]),
        "later commit\nimproved initial"
    );
    assert_ne!(r.head().unwrap(), head);
    assert_eq!(
        r.blob("HEAD~1:file.txt").unwrap(),
        b"ONE\ntwo\nthree\nfour\nfive\n"
    );
    assert_eq!(r.blob("HEAD:later.txt").unwrap(), b"later\n");
    assert_eq!(
        fs::read(r.root.join("file.txt")).unwrap(),
        b"ONE\ntwo\nthree\nfour\nFIVE\n"
    );
    assert!(git(&r.root, &["diff", "--cached"]).is_empty());
    assert_eq!(
        git(&r.root, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert!(
        git(
            &r.root,
            &[
                "for-each-ref",
                "--format=%(objectname)",
                "refs/difly/before-amend"
            ]
        )
        .contains(&head)
    );
}

#[test]
fn older_message_only_amend_preserves_authors_and_newer_changes() {
    let (_temp, r) = repo();
    seed(&r);
    let target = r.head().unwrap();
    git(&r.root, &["config", "user.name", "Second Author"]);
    fs::write(r.root.join("file.txt"), "later content\n").unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "later"]);
    let head = r.head().unwrap();
    r.amend_commit(&[], "renamed", &target, Some(&head))
        .unwrap();
    assert_eq!(
        git(&r.root, &["log", "--format=%an:%s"]),
        "Second Author:later\nDifly Test:renamed"
    );
    assert_eq!(r.blob("HEAD:file.txt").unwrap(), b"later content\n");
    assert_eq!(r.recent_commits().unwrap()[1].message, "renamed");
}

#[test]
fn older_amend_conflict_leaves_branch_index_and_worktree_untouched() {
    let (_temp, r) = repo();
    seed(&r);
    let target = r.head().unwrap();
    fs::write(r.root.join("file.txt"), "later\n").unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "later"]);
    let head = r.head().unwrap();
    fs::write(r.root.join("file.txt"), "new edit\n").unwrap();
    git(&r.root, &["add", "."]);
    let index = git(&r.root, &["write-tree"]);
    let files = r.scan().unwrap();
    assert!(
        r.amend_commit(&files, "conflicting", &target, Some(&head))
            .is_err()
    );
    assert_eq!(r.head().unwrap(), head);
    assert_eq!(git(&r.root, &["write-tree"]), index);
    assert_eq!(fs::read(r.root.join("file.txt")).unwrap(), b"new edit\n");
    assert_eq!(
        git(&r.root, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn older_amend_rejects_stale_head_and_unrelated_target() {
    let (_temp, r) = repo();
    seed(&r);
    let target = r.head().unwrap();
    git(&r.root, &["commit", "--allow-empty", "-m", "later"]);
    let head = r.head().unwrap();
    assert!(
        r.amend_commit(&[], "stale", &target, Some(&target))
            .is_err()
    );
    assert!(
        r.amend_commit(
            &[],
            "invalid",
            "0000000000000000000000000000000000000000",
            Some(&head)
        )
        .is_err()
    );
    assert_eq!(r.head().unwrap(), head);
}

#[test]
fn older_amend_rejects_merge_history_without_changes() {
    let (_temp, r) = repo();
    seed(&r);
    let target = r.head().unwrap();
    git(&r.root, &["checkout", "-b", "side"]);
    fs::write(r.root.join("side.txt"), "side").unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "side"]);
    git(&r.root, &["checkout", "main"]);
    git(&r.root, &["merge", "--no-ff", "side", "-m", "merge"]);
    let head = r.head().unwrap();
    let err = r
        .amend_commit(&[], "change", &target, Some(&head))
        .unwrap_err();
    assert!(err.to_string().contains("across merges"));
    assert_eq!(r.head().unwrap(), head);
}

#[test]
#[cfg(unix)]
fn older_amend_hook_failure_preserves_original_history() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, r) = repo();
    seed(&r);
    let target = r.head().unwrap();
    git(&r.root, &["commit", "--allow-empty", "-m", "later"]);
    let head = r.head().unwrap();
    let hook = r.root.join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        r.amend_commit(&[], "changed", &target, Some(&head))
            .is_err()
    );
    assert_eq!(r.head().unwrap(), head);
    assert_eq!(
        git(&r.root, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn metadata_scan_and_binary_preview_do_not_retain_payloads() {
    let (_temp, r) = repo();
    seed(&r);
    for name in ["a.bin", "b.bin", "c.bin"] {
        fs::write(r.root.join(name), vec![0; 1024 * 1024]).unwrap();
    }
    let mut files = r.scan_metadata().unwrap();
    assert_eq!(files.len(), 3);
    for file in &mut files {
        assert!(!file.loaded && !file.included());
        assert!(file.old.is_empty() && file.current.is_empty());
        r.load_preview(file).unwrap();
        assert!(file.binary && !file.loaded);
        assert_eq!(file.sizes(), (0, 1024 * 1024));
    }
    files[0].select(true);
    r.commit(&files, "one binary", false, r.head().as_deref())
        .unwrap();
    assert_eq!(r.blob("HEAD:a.bin").unwrap().len(), 1024 * 1024);
    assert!(r.blob("HEAD:b.bin").is_err());
}
#[test]
fn unloaded_partial_selection_survives_reopen_and_commit() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "ONE\ntwo\nthree\nfour\nFIVE\n").unwrap();
    let mut files = r.scan_metadata().unwrap();
    r.load_file(&mut files[0]).unwrap();
    files[0].select(false);
    files[0].hunks[0].selected = true;
    files[0].unload();
    assert!(files[0].old.is_empty() && files[0].current.is_empty());
    assert!(
        files[0]
            .hunks
            .iter()
            .all(|h| h.added.is_empty() && h.removed.is_empty())
    );
    r.load_file(&mut files[0]).unwrap();
    assert!(files[0].hunks[0].selected && !files[0].hunks[1].selected);
    files[0].unload();
    r.commit(&files, "partial lazy", false, r.head().as_deref())
        .unwrap();
    assert_eq!(
        r.blob("HEAD:file.txt").unwrap(),
        b"ONE\ntwo\nthree\nfour\nfive\n"
    );
}
#[test]
fn lazy_operations_reject_external_changes() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "first change\n").unwrap();
    let mut files = r.scan_metadata().unwrap();
    let head = r.head();
    fs::write(r.root.join("file.txt"), "later change\n").unwrap();
    assert!(r.load_file(&mut files[0]).is_err());
    assert!(r.commit(&files, "stale", false, head.as_deref()).is_err());
    assert_eq!(r.head(), head);
}
#[test]
fn lazy_autosave_updates_snapshot_and_preserves_exclusion() {
    let (_temp, r) = repo();
    seed(&r);
    fs::write(r.root.join("file.txt"), "ONE\ntwo\nthree\nfour\nfive\n").unwrap();
    let mut file = r.scan_metadata().unwrap().remove(0);
    r.load_file(&mut file).unwrap();
    file.select(false);
    r.save(&mut file, "EDITED\ntwo\nthree\nfour\nfive\n")
        .unwrap();
    file.unload();
    r.load_file(&mut file).unwrap();
    assert!(!file.included());
    assert!(file.text().starts_with("EDITED"));
}
#[test]
fn metadata_scan_ignores_staged_content_restored_in_worktree() {
    let (_temp, r) = repo();
    seed(&r);
    let original = fs::read(r.root.join("file.txt")).unwrap();
    fs::write(r.root.join("file.txt"), "staged\n").unwrap();
    git(&r.root, &["add", "file.txt"]);
    fs::write(r.root.join("file.txt"), original).unwrap();
    assert!(r.scan_metadata().unwrap().is_empty());
}
#[test]
fn lazy_deletion_and_untracked_initial_commit() {
    let (_temp, r) = repo();
    fs::write(r.root.join("new.txt"), "new\n").unwrap();
    let mut files = r.scan_metadata().unwrap();
    files[0].select(true);
    r.commit(&files, "initial lazy", false, None).unwrap();
    fs::remove_file(r.root.join("new.txt")).unwrap();
    let files = r.scan_metadata().unwrap();
    r.commit(&files, "delete lazy", false, r.head().as_deref())
        .unwrap();
    assert!(r.scan_metadata().unwrap().is_empty());
    assert!(r.blob("HEAD:new.txt").is_err());
}

#[test]
fn lazy_older_amend_replays_history_without_loading_all_files() {
    let (_temp, r) = repo();
    seed(&r);
    let target = r.head().unwrap();
    fs::write(r.root.join("other.txt"), "later\n").unwrap();
    git(&r.root, &["add", "."]);
    git(&r.root, &["commit", "-m", "later"]);
    fs::write(r.root.join("file.txt"), "ONE\ntwo\nthree\nfour\nfive\n").unwrap();
    let files = r.scan_metadata().unwrap();
    assert!(files.iter().all(|f| !f.loaded));
    r.amend_commit(&files, "amended", &target, r.head().as_deref())
        .unwrap();
    assert_eq!(
        r.blob("HEAD~1:file.txt").unwrap(),
        b"ONE\ntwo\nthree\nfour\nfive\n"
    );
    assert_eq!(r.blob("HEAD:other.txt").unwrap(), b"later\n");
}
#[test]
fn lazy_merge_resolution_and_commit_preserve_two_parents() {
    let (_temp, r) = repo();
    seed(&r);
    git(&r.root, &["checkout", "-b", "side"]);
    fs::write(r.root.join("file.txt"), "side\n").unwrap();
    git(&r.root, &["commit", "-am", "side"]);
    git(&r.root, &["checkout", "main"]);
    fs::write(r.root.join("file.txt"), "main\n").unwrap();
    git(&r.root, &["commit", "-am", "main"]);
    let output = Command::new("git")
        .arg("-C")
        .arg(&r.root)
        .args(["merge", "side"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let mut file = r.scan_metadata().unwrap().remove(0);
    assert!(file.conflict);
    r.load_file(&mut file).unwrap();
    r.save(&mut file, "resolved\n").unwrap();
    r.resolve(&file).unwrap();
    let files = r.scan_metadata().unwrap();
    r.commit(&files, "merge lazy", false, r.head().as_deref())
        .unwrap();
    assert_eq!(
        git(&r.root, &["rev-list", "--parents", "-1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
    assert_eq!(r.blob("HEAD:file.txt").unwrap(), b"resolved\n");
}
