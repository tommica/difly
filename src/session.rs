//! Repository session and commit workflow, independent of the GUI toolkit.
use crate::{
    git::{CommitInfo, File, Repo},
    model,
};
use anyhow::{Context, Result, bail};
use std::{ops::Range, path::Path};

#[derive(Clone, Debug, Default)]
pub struct CommitDraft {
    pub message: String,
    pub target: Option<String>,
    previous_message: Option<String>,
}
impl CommitDraft {
    pub fn amend(&mut self, commit: Option<&CommitInfo>) {
        match commit {
            Some(commit) => {
                if self.target.is_none() {
                    self.previous_message = Some(self.message.clone());
                }
                self.target = Some(commit.oid.clone());
                self.message = commit.message.clone();
            }
            None => {
                if self.target.take().is_some() {
                    self.message = self.previous_message.take().unwrap_or_default();
                }
            }
        }
    }
    pub fn normal_message(&self) -> &str {
        self.previous_message.as_deref().unwrap_or(&self.message)
    }
}
pub struct CommitRequest {
    repo: Repo,
    files: Vec<File>,
    message: String,
    target: Option<String>,
    head: Option<String>,
}
impl CommitRequest {
    pub fn execute(self) -> Result<String> {
        if let Some(target) = self.target {
            self.repo
                .amend_commit(&self.files, &self.message, &target, self.head.as_deref())
        } else {
            self.repo
                .commit(&self.files, &self.message, false, self.head.as_deref())
        }
    }
}
struct UndoRevert {
    after: File,
    before: Option<Vec<u8>>,
    mode: String,
    selected: bool,
    hunks: Vec<model::Hunk>,
}

pub struct Session {
    pub repo: Repo,
    pub files: Vec<File>,
    pub active: Option<usize>,
    pub cursor: Option<usize>,
    pub head: Option<String>,
    pub recent_commits: Vec<CommitInfo>,
    pub draft: CommitDraft,
    undo: Vec<UndoRevert>,
}
impl Session {
    pub fn open(path: &Path) -> Result<Self> {
        let repo = Repo::open(path)?;
        let files = repo.scan_metadata()?;
        let head = repo.head();
        let recent_commits = repo.recent_commits()?;
        let draft = CommitDraft {
            message: if repo.merge() {
                repo.merge_message()
            } else {
                String::new()
            },
            ..Default::default()
        };
        Ok(Self {
            head,
            recent_commits,
            draft,
            undo: vec![],
            repo,
            files,
            active: None,
            cursor: None,
        })
    }
    pub fn activate(&mut self, index: usize) -> Result<()> {
        // Load first: a failed switch must not lose the current document.
        let file = self
            .files
            .get_mut(index)
            .ok_or_else(|| anyhow::anyhow!("Unknown file"))?;
        self.repo.load_preview(file)?;
        if let Some(previous) = self.active.filter(|&i| i != index) {
            self.files[previous].unload();
        }
        self.active = Some(index);
        self.cursor = None;
        Ok(())
    }
    pub fn file(&self) -> Option<&File> {
        self.active.and_then(|i| self.files.get(i))
    }
    pub fn save(&mut self, text: &str) -> Result<()> {
        let Some(i) = self.active else {
            bail!("No active file")
        };
        if self.files[i].text() != text {
            self.repo.save(&mut self.files[i], text)?;
        } else {
            let file = &self.files[i];
            anyhow::ensure!(
                self.repo.read_work(&file.path)? == file.exists.then(|| file.current.clone()),
                "{} changed outside Difly. Reload before saving.",
                file.path.display()
            );
        }
        Ok(())
    }
    pub fn select_hunk(&mut self, hunk: usize, selected: bool) -> Result<()> {
        let i = self.active.context("No active file")?;
        let file = &mut self.files[i];
        let hunk = file
            .hunks
            .get_mut(hunk)
            .context("The hunk changed; select it again")?;
        hunk.selected = selected;
        hunk.review = false;
        file.selected = file.hunks.iter().all(|h| h.selected);
        Ok(())
    }
    pub fn jump(&mut self, backwards: bool) -> Option<(Range<usize>, Range<usize>)> {
        let file = self.file()?;
        let count = file.hunks.len();
        if count == 0 {
            self.cursor = None;
            return None;
        }
        let i = match self.cursor {
            None => {
                if backwards {
                    count - 1
                } else {
                    0
                }
            }
            Some(i) => {
                if backwards {
                    i.min(count - 1).saturating_sub(1)
                } else {
                    (i + 1).min(count - 1)
                }
            }
        };
        self.cursor = Some(i);
        let file = self.file()?;
        let h = &file.hunks[i];
        Some((
            byte_range(&file.old_text(), h.old.clone()),
            byte_range(&file.text(), h.new.clone()),
        ))
    }
    pub fn refresh(&mut self) -> Result<()> {
        let mut files = self.repo.scan_metadata()?;
        let head = self.repo.head();
        if head == self.head {
            for file in &mut files {
                if let Some(previous) = self.files.iter().find(|p| p.path == file.path)
                    && previous.source.as_ref().map(|s| &s.oid)
                        == file.source.as_ref().map(|s| &s.oid)
                {
                    file.selected = previous.selected;
                    file.hunks = previous.hunks.clone();
                }
            }
        }
        if self.repo.merge() {
            self.draft.amend(None);
            if self.draft.message.is_empty() {
                self.draft.message = self.repo.merge_message();
            }
        }
        self.head = head;
        self.recent_commits = self.repo.recent_commits()?;
        self.files = files;
        self.active = None;
        self.cursor = None;
        Ok(())
    }
    pub fn commit_request(&self) -> Result<CommitRequest> {
        anyhow::ensure!(
            !self.draft.message.trim().is_empty(),
            "Enter a commit message"
        );
        Ok(CommitRequest {
            repo: self.repo.clone(),
            files: self.files.clone(),
            message: self.draft.message.clone(),
            target: self.draft.target.clone(),
            head: self.head.clone(),
        })
    }
    pub fn committed(&mut self) -> Result<()> {
        self.draft = CommitDraft::default();
        self.undo.clear();
        self.refresh()
    }
    fn restore(&mut self, bytes: Option<Vec<u8>>, mode: String) -> Result<()> {
        let i = self.active.context("No active file")?;
        self.repo.load_file(&mut self.files[i])?;
        let file = &self.files[i];
        let before = file.exists.then(|| file.current.clone());
        let previous_mode = file.mode.clone();
        let selected = file.selected;
        let mut hunks = file.hunks.clone();
        for h in &mut hunks {
            h.added = String::new();
            h.removed = String::new();
        }
        self.repo.restore_work(file, bytes.as_deref(), &mode)?;
        let mut after = file.clone();
        after.exists = bytes.is_some();
        after.current = bytes.unwrap_or_default();
        after.mode = mode;
        after.source = None;
        after.old = Vec::new();
        after.hunks = Vec::new();
        self.undo.push(UndoRevert {
            after,
            before,
            mode: previous_mode,
            selected,
            hunks,
        });
        while self.undo.len() > 1
            && (self.undo.len() > 20
                || self
                    .undo
                    .iter()
                    .map(|u| u.after.current.len() + u.before.as_ref().map_or(0, Vec::len))
                    .sum::<usize>()
                    > 32 * 1024 * 1024)
        {
            self.undo.remove(0);
        }
        let path = self.files[i].path.clone();
        self.refresh()?;
        if let Some(i) = self.files.iter().position(|f| f.path == path) {
            self.activate(i)?;
        }
        Ok(())
    }
    pub fn revert(&mut self, index: usize) -> Result<String> {
        let file = self.file().context("No active file")?;
        let hunk = file.hunks.get(index).context("Unknown hunk")?;
        let text = model::revert_hunk(&file.text(), hunk);
        let mode = file.mode.clone();
        self.restore(Some(text.clone().into_bytes()), mode)?;
        Ok(text)
    }
    pub fn revert_file(&mut self) -> Result<()> {
        let i = self.active.context("No active file")?;
        self.repo.load_file(&mut self.files[i])?;
        let file = &self.files[i];
        self.restore(
            file.base_exists.then(|| file.old.clone()),
            file.old_mode.clone(),
        )
    }
    pub fn can_undo_revert(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn undo_revert(&mut self) -> Result<()> {
        let undo = self.undo.last().context("Nothing to undo")?;
        self.repo
            .restore_work(&undo.after, undo.before.as_deref(), &undo.mode)?;
        let path = undo.after.path.clone();
        let undo = self.undo.pop().unwrap();
        self.refresh()?;
        if let Some(i) = self.files.iter().position(|f| f.path == path) {
            self.files[i].selected = undo.selected;
            self.files[i].hunks = undo.hunks;
            self.activate(i)?;
        }
        Ok(())
    }
    pub fn accept_conflict(&mut self, stage: u8) -> Result<()> {
        anyhow::ensure!(stage == 2 || stage == 3, "Choose ours or theirs");
        let file = self.file().context("No active file")?;
        anyhow::ensure!(file.conflict, "No conflict in this file");
        let bytes = self.repo.conflict_blob(&file.path, stage)?;
        self.restore(bytes, file.mode.clone())
    }
    pub fn resolve(&mut self) -> Result<()> {
        let i = self.active.context("No active file")?;
        self.repo.load_file(&mut self.files[i])?;
        self.repo.resolve(&self.files[i])?;
        self.refresh()
    }
}
pub fn byte_range(text: &str, lines: Range<usize>) -> Range<usize> {
    let at = |line| text.split_inclusive('\n').take(line).map(str::len).sum();
    at(lines.start)..at(lines.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    fn fixture() -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(dir.path())
                    .output()
                    .unwrap()
                    .status
                    .success()
            )
        };
        git(&["init", "-q"]);
        fs::write(dir.path().join("code.rs"), "one\nkeep\nthree\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Initial",
        ]);
        fs::write(dir.path().join("code.rs"), "ONE\nkeep\nTHREE\n").unwrap();
        let mut s = Session::open(dir.path()).unwrap();
        s.activate(0).unwrap();
        (dir, s)
    }
    #[test]
    fn autosave_preserves_excluded_hunks_and_refuses_external_overwrite() {
        let (dir, mut s) = fixture();
        s.files[0].hunks[0].selected = false;
        s.save("ONE edited\nkeep\nTHREE\n").unwrap();
        assert!(!s.files[0].hunks[0].selected);
        assert!(s.files[0].hunks[1].selected);
        fs::write(dir.path().join("code.rs"), "external\n").unwrap();
        assert!(s.save("overwrite\n").is_err());
        // Undoing to the last loaded value must not falsely clear a failed save.
        assert!(s.save("ONE edited\nkeep\nTHREE\n").is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("code.rs")).unwrap(),
            "external\n"
        );
    }
    #[test]
    fn navigation_and_revert_use_shared_hunks_and_utf8_byte_ranges() {
        let (_dir, mut s) = fixture();
        assert_eq!(s.jump(false), Some((0..4, 0..4)));
        assert_eq!(s.jump(false), Some((9..15, 9..15)));
        assert_eq!(s.revert(0).unwrap(), "one\nkeep\nTHREE\n");
        assert_eq!(byte_range("😃\n終\n", 1..2), 5..9);
    }
    #[test]
    fn a_partial_commit_leaves_the_other_hunk_on_disk_and_unstaged() {
        let (dir, mut session) = fixture();
        session.files[0].hunks[1].selected = false;
        session.files[0].selected = false;
        session.draft.message = "Only the first hunk".into();
        session.commit_request().unwrap().execute().unwrap();
        assert_eq!(
            String::from_utf8(session.repo.git(&["show", "HEAD:code.rs"]).unwrap()).unwrap(),
            "ONE\nkeep\nthree\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("code.rs")).unwrap(),
            "ONE\nkeep\nTHREE\n"
        );
        assert!(session.repo.git(&["diff", "--cached"]).unwrap().is_empty());
        session.committed().unwrap();
        assert!(session.draft.message.is_empty());
    }
    #[test]
    fn amend_target_changes_restore_the_original_message_when_disabled() {
        let mut draft = CommitDraft {
            message: "My unfinished message".into(),
            ..Default::default()
        };
        draft.amend(Some(&CommitInfo {
            oid: "first".into(),
            message: "First commit".into(),
        }));
        draft.message = "Edited amendment".into();
        draft.amend(Some(&CommitInfo {
            oid: "second".into(),
            message: "Second commit".into(),
        }));
        assert_eq!(draft.normal_message(), "My unfinished message");
        draft.amend(None);
        assert_eq!(draft.message, "My unfinished message");
        assert!(draft.target.is_none());
    }
    #[test]
    fn revert_undo_restores_bytes_and_rejects_external_edits() {
        let (dir, mut session) = fixture();
        session.revert(0).unwrap();
        session.undo_revert().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("code.rs")).unwrap(),
            "ONE\nkeep\nTHREE\n"
        );
        session.revert(0).unwrap();
        fs::write(dir.path().join("code.rs"), "external\n").unwrap();
        assert!(session.undo_revert().is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("code.rs")).unwrap(),
            "external\n"
        );
    }
    #[test]
    fn an_unversioned_file_can_be_reverted_and_restored_without_losing_its_bytes() {
        let (dir, mut session) = fixture();
        fs::write(dir.path().join("new.txt"), "unversioned\n").unwrap();
        session.refresh().unwrap();
        let i = session
            .files
            .iter()
            .position(|f| f.path == Path::new("new.txt"))
            .unwrap();
        session.activate(i).unwrap();
        session.revert_file().unwrap();
        assert!(!dir.path().join("new.txt").exists());
        session.undo_revert().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "unversioned\n"
        );
    }
    #[test]
    fn a_commit_request_refuses_a_head_that_changed_after_review() {
        let (_dir, mut session) = fixture();
        session.draft.message = "Reviewed".into();
        let request = session.commit_request().unwrap();
        session
            .repo
            .git(&[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "External commit",
            ])
            .unwrap();
        assert!(request.execute().is_err());
    }
    #[test]
    fn undo_revert_restores_hunk_inclusion_and_refresh_preserves_partial_selection() {
        let (_dir, mut session) = fixture();
        session.select_hunk(1, false).unwrap();
        session.revert(0).unwrap();
        session.undo_revert().unwrap();
        let file = session.file().unwrap();
        assert!(file.hunks[0].selected);
        assert!(!file.hunks[1].selected);
        session.refresh().unwrap();
        session.activate(0).unwrap();
        assert!(session.file().unwrap().hunks[0].selected);
        assert!(!session.file().unwrap().hunks[1].selected);
        assert!(session.select_hunk(100, true).is_err());
    }
    #[test]
    fn session_can_choose_a_conflict_version_resolve_and_complete_a_merge() {
        let (dir, session) = fixture();
        let repo = session.repo;
        fs::write(dir.path().join("code.rs"), "one\nkeep\nthree\n").unwrap();
        repo.git(&["config", "user.name", "Test"]).unwrap();
        repo.git(&["config", "user.email", "test@example.com"])
            .unwrap();
        repo.git(&["config", "commit.gpgsign", "false"]).unwrap();
        let branch = repo.branch();
        repo.git(&["checkout", "-b", "other"]).unwrap();
        fs::write(dir.path().join("code.rs"), "theirs\n").unwrap();
        repo.git(&["add", "."]).unwrap();
        repo.git(&["commit", "-m", "Other"]).unwrap();
        repo.git(&["checkout", &branch]).unwrap();
        fs::write(dir.path().join("code.rs"), "ours\n").unwrap();
        repo.git(&["add", "."]).unwrap();
        repo.git(&["commit", "-m", "Ours"]).unwrap();
        assert!(repo.git(&["merge", "other"]).is_err());
        let mut session = Session::open(dir.path()).unwrap();
        session.activate(0).unwrap();
        assert!(session.file().unwrap().conflict);
        assert!(session.resolve().is_err());
        session.accept_conflict(2).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("code.rs")).unwrap(),
            "ours\n"
        );
        session.resolve().unwrap();
        assert!(!session.files.iter().any(|f| f.conflict));
        session.commit_request().unwrap().execute().unwrap();
        assert!(!session.repo.merge());
        assert_eq!(
            String::from_utf8(
                session
                    .repo
                    .git(&["rev-list", "--parents", "-n", "1", "HEAD"])
                    .unwrap()
            )
            .unwrap()
            .split_whitespace()
            .count(),
            3
        );
    }
}
