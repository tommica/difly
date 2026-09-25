use crate::model::{self, Hunk};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug)]
pub struct Repo {
    pub root: PathBuf,
}
#[derive(Clone, Debug)]
pub struct CommitInfo {
    pub oid: String,
    pub message: String,
}
impl CommitInfo {
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or("(no message)")
    }
}

/// Identity of an unopened file. No file payloads are retained by repository scans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSource {
    pub oid: Option<String>,
    pub old_size: u64,
    pub work: WorkStamp,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkStamp {
    pub size: u64,
    modified: Option<std::time::SystemTime>,
    exists: bool,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

#[derive(Clone, Debug)]
pub struct File {
    pub path: PathBuf,
    pub old: Vec<u8>,
    pub current: Vec<u8>,
    pub exists: bool,
    pub base_exists: bool,
    pub tracked: bool,
    pub conflict: bool,
    pub binary: bool,
    pub mode: String,
    pub old_mode: String,
    pub hunks: Vec<Hunk>,
    pub selected: bool,
    pub loaded: bool,
    pub source: Option<FileSource>,
}
impl File {
    pub fn sizes(&self) -> (u64, u64) {
        if self.loaded {
            (self.old.len() as u64, self.current.len() as u64)
        } else {
            self.source
                .as_ref()
                .map(|s| (s.old_size, s.work.size))
                .unwrap_or_default()
        }
    }
    pub fn unload(&mut self) {
        if self.source.is_some() {
            self.old = Vec::new();
            self.current = Vec::new();
            for h in &mut self.hunks {
                h.added = String::new();
                h.removed = String::new();
            }
            self.loaded = false;
        }
    }

    pub fn old_text(&self) -> String {
        String::from_utf8_lossy(&self.old).into_owned()
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.current).into_owned()
    }
    pub fn included(&self) -> bool {
        self.selected || self.hunks.iter().any(|h| h.selected)
    }
    pub fn select(&mut self, value: bool) {
        self.selected = value;
        for h in &mut self.hunks {
            h.selected = value;
            h.review = false;
        }
    }
}

impl Repo {
    pub fn open(path: &Path) -> Result<Self> {
        let out = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Git must be installed and on PATH")?;
        ensure!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let root = bytes_path(out.stdout.strip_suffix(b"\n").unwrap_or(&out.stdout));
        Ok(Self { root })
    }
    pub fn command(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("--literal-pathspecs").arg("-C").arg(&self.root);
        c.env("GIT_TERMINAL_PROMPT", "0");
        c.env("GIT_EDITOR", "true");
        c
    }
    pub fn run(&self, args: &[&OsStr]) -> Result<Vec<u8>> {
        let out = self.command().args(args).output()?;
        ensure!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(out.stdout)
    }
    pub fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        self.run(&args.iter().map(OsStr::new).collect::<Vec<_>>())
    }
    pub fn head(&self) -> Option<String> {
        self.git(&["rev-parse", "--verify", "HEAD"])
            .ok()
            .map(|v| String::from_utf8_lossy(&v).trim().into())
    }
    pub fn branch(&self) -> String {
        self.git(&["symbolic-ref", "--short", "HEAD"])
            .map(|v| String::from_utf8_lossy(&v).trim().into())
            .unwrap_or_else(|_| "detached HEAD".into())
    }
    pub fn merge(&self) -> bool {
        self.git(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .is_ok()
    }
    pub fn message(&self) -> String {
        self.git(&["log", "-1", "--format=%B"])
            .map(|v| String::from_utf8_lossy(&v).trim_end().into())
            .unwrap_or_default()
    }
    pub fn recent_commits(&self) -> Result<Vec<CommitInfo>> {
        if self.head().is_none() {
            return Ok(vec![]);
        }
        let data = self.git(&["log", "--first-parent", "-40", "--format=%H%x00%B%x00"])?;
        let fields: Vec<_> = data.split(|&b| b == 0).collect();
        Ok(fields
            .as_chunks::<2>()
            .0
            .iter()
            .map(|parts| CommitInfo {
                oid: String::from_utf8_lossy(parts[0]).trim().into(),
                message: String::from_utf8_lossy(parts[1]).trim_end().into(),
            })
            .collect())
    }

    /// Rewrite on a detached worktree. Publish only after every replay succeeds.
    pub fn amend_commit(
        &self,
        files: &[File],
        message: &str,
        target: &str,
        expected_head: Option<&str>,
    ) -> Result<String> {
        let head = expected_head.context("No commit to amend")?;
        if target == head {
            return self.commit(files, message, true, expected_head);
        }
        ensure!(!self.merge(), "Cannot rewrite history during a merge");
        let branch = String::from_utf8(self.git(&["symbolic-ref", "HEAD"])?)?
            .trim()
            .to_owned();
        let ancestry = String::from_utf8(self.git(&["rev-list", "--first-parent", head])?)?;
        ensure!(
            ancestry.lines().any(|id| id == target),
            "Selected commit is no longer on this branch; reopen the commit picker"
        );
        let range = format!("{target}..{head}");
        let descendants =
            String::from_utf8(self.git(&["rev-list", "--reverse", "--first-parent", &range])?)?;
        let mut commits = vec![target.to_owned()];
        commits.extend(descendants.lines().map(str::to_owned));
        for oid in &commits {
            let parents = self.git(&["rev-list", "--parents", "-1", oid])?;
            ensure!(
                String::from_utf8_lossy(&parents).split_whitespace().count() <= 2,
                "Amending older commits across merges is not supported yet"
            );
        }
        let tree = self.commit_impl(files, message, true, expected_head, true)?;
        let temp = tempfile::tempdir()?;
        let msg = temp.path().join("message");
        fs::write(&msg, message)?;
        let snapshot = String::from_utf8(self.run(&[
            OsStr::new("commit-tree"),
            OsStr::new(&tree),
            OsStr::new("-p"),
            OsStr::new(head),
            OsStr::new("-F"),
            msg.as_os_str(),
        ])?)?
        .trim()
        .to_owned();
        let work_path = temp.path().join("rewrite");
        self.run(&[
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("--detach"),
            work_path.as_os_str(),
            OsStr::new(target),
        ])?;
        let result = (|| -> Result<String> {
            let work = Repo::open(&work_path)?;
            if tree
                != String::from_utf8(self.git(&["rev-parse", &format!("{head}^{{tree}}")])?)?.trim()
            {
                work.git(&["cherry-pick", "--no-commit", &snapshot])
                    .context("Selected changes conflict with the older commit. Branch and working files were left unchanged")?;
            }
            work.run(&[
                OsStr::new("commit"),
                OsStr::new("--amend"),
                OsStr::new("--allow-empty"),
                OsStr::new("-F"),
                msg.as_os_str(),
            ])?;
            for oid in descendants.lines() {
                work.git(&["cherry-pick", "--allow-empty", oid])
                    .context("Replaying newer commits caused a conflict. Branch and working files were left unchanged")?;
            }
            let rewritten = work.head().context("Rewritten HEAD missing")?;
            let final_tree = String::from_utf8(work.git(&["rev-parse", "HEAD^{tree}"])?)?;
            ensure!(
                final_tree.trim() == tree,
                "Rewrite changed the resulting file tree unexpectedly; branch was left unchanged"
            );
            ensure!(
                self.head().as_deref() == Some(head)
                    && String::from_utf8(self.git(&["symbolic-ref", "HEAD"])?)?.trim() == branch,
                "Branch changed during rewrite; nothing was published"
            );
            for f in files {
                self.ensure_current(f)?;
            }
            // Keep the original chain reachable for recovery, then compare-and-swap the branch.
            let backup = format!("refs/difly/before-amend/{rewritten}");
            self.git(&["update-ref", &backup, head])?;
            self.git(&[
                "update-ref",
                "-m",
                "difly: amend older commit",
                &branch,
                &rewritten,
                head,
            ])?;
            self.git(&["read-tree", &rewritten]).context(
                "Amend succeeded, but staging could not be normalized. Run git reset --mixed HEAD",
            )?;
            Ok(format!(
                "Amended {} and replayed {} newer commits",
                &target[..8],
                commits.len() - 1
            ))
        })();
        let cleanup = self.run(&[
            OsStr::new("worktree"),
            OsStr::new("remove"),
            OsStr::new("--force"),
            work_path.as_os_str(),
        ]);
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(message), Err(error)) => Ok(format!(
                "{message}. Temporary worktree cleanup failed: {error}"
            )),
            (Ok(message), Ok(_)) => Ok(message),
        }
    }

    pub fn merge_message(&self) -> String {
        self.git(&["rev-parse", "--git-path", "MERGE_MSG"])
            .ok()
            .and_then(|p| {
                fs::read_to_string(
                    self.root
                        .join(bytes_path(p.strip_suffix(b"\n").unwrap_or(&p))),
                )
                .ok()
            })
            .unwrap_or_default()
    }
    pub fn blob(&self, spec: &str) -> Result<Vec<u8>> {
        self.git(&["show", spec])
    }
    fn head_entry(&self, path: &Path) -> Result<Option<(String, Vec<u8>)>> {
        if self.head().is_none() {
            return Ok(None);
        }
        let data = self.run(&[
            OsStr::new("ls-tree"),
            OsStr::new("-z"),
            OsStr::new("HEAD"),
            OsStr::new("--"),
            path.as_os_str(),
        ])?;
        if data.is_empty() {
            return Ok(None);
        }
        let header = data.split(|&b| b == b'\t').next().unwrap();
        let fields: Vec<_> = header.split(|&b| b == b' ').collect();
        let mode = String::from_utf8_lossy(fields[0]).to_string();
        ensure!(
            mode != "160000",
            "Submodules are not supported: {}",
            path.display()
        );
        let oid = String::from_utf8_lossy(fields[2]);
        let content = if mode == "120000" {
            self.git(&["cat-file", "blob", &oid])?
        } else {
            let mut path_arg = OsString::from("--path=");
            path_arg.push(path);
            self.run(&[
                OsStr::new("cat-file"),
                OsStr::new("--filters"),
                &path_arg,
                OsStr::new(oid.as_ref()),
            ])?
        };
        Ok(Some((mode, content)))
    }
    pub fn read_work(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        let full = self.safe_path(path)?;
        match fs::symlink_metadata(&full) {
            Ok(meta) if meta.file_type().is_symlink() => {
                Ok(Some(path_bytes(&fs::read_link(full)?)))
            }
            Ok(meta) if meta.is_file() => Ok(Some(fs::read(full)?)),
            Ok(_) => bail!("Unsupported file type: {}", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn safe_path(&self, path: &Path) -> Result<PathBuf> {
        ensure!(
            !path.is_absolute()
                && path
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Unsafe repository path"
        );
        let full = self.root.join(path);
        let mut parent = full.parent();
        while let Some(p) = parent {
            if p == self.root {
                break;
            }
            if p.exists() {
                ensure!(
                    !fs::symlink_metadata(p)?.file_type().is_symlink(),
                    "Refusing to traverse a symlink: {}",
                    p.display()
                );
            }
            parent = p.parent();
        }
        Ok(full)
    }
    pub fn scan_metadata(&self) -> Result<Vec<File>> {
        let status = self.git(&[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--no-renames",
        ])?;
        let tree = if self.head().is_some() {
            self.git(&["ls-tree", "-r", "-l", "-z", "HEAD"])?
        } else {
            vec![]
        };
        let mut entries = std::collections::HashMap::new();
        for record in tree.split(|&b| b == 0).filter(|r| !r.is_empty()) {
            let Some(tab) = record.iter().position(|&b| b == b'\t') else {
                continue;
            };
            let fields: Vec<_> = record[..tab]
                .split(|b| b.is_ascii_whitespace())
                .filter(|s| !s.is_empty())
                .collect();
            entries.insert(
                bytes_path(&record[tab + 1..]),
                (
                    String::from_utf8_lossy(fields[0]).into_owned(),
                    String::from_utf8_lossy(fields[2]).into_owned(),
                    std::str::from_utf8(fields[3])
                        .unwrap_or("0")
                        .parse::<u64>()
                        .unwrap_or(0),
                ),
            );
        }
        let changed = if self.head().is_some() {
            Some(
                self.git(&["diff", "--name-only", "-z", "HEAD", "--"])?
                    .split(|&b| b == 0)
                    .filter(|p| !p.is_empty())
                    .map(bytes_path)
                    .collect::<std::collections::HashSet<_>>(),
            )
        } else {
            None
        };
        let mut files = Vec::new();
        for record in status.split(|&b| b == 0).filter(|r| r.len() > 3) {
            let code = &record[..2];
            let path = bytes_path(&record[3..]);
            if code != b"??"
                && !code.contains(&b'U')
                && code != b"AA"
                && code != b"DD"
                && changed.as_ref().is_some_and(|paths| !paths.contains(&path))
            {
                continue;
            }
            let entry = entries.get(&path);
            let old_mode = entry
                .map(|e| e.0.clone())
                .unwrap_or_else(|| "100644".into());
            ensure!(
                old_mode != "160000",
                "Submodules are not supported: {}",
                path.display()
            );
            let full = self.safe_path(&path)?;
            let work = work_stamp(&full)?;
            let mode = work_mode(&full, &old_mode);
            let tracked = code != b"??";
            let conflict = code.contains(&b'U') || code == b"AA" || code == b"DD";
            files.push(File {
                path,
                old: vec![],
                current: vec![],
                exists: work.exists,
                base_exists: entry.is_some(),
                tracked,
                conflict,
                binary: mode == "120000" || old_mode == "120000",
                mode,
                old_mode,
                hunks: vec![],
                selected: tracked && !conflict,
                loaded: false,
                source: Some(FileSource {
                    oid: entry.map(|e| e.1.clone()),
                    old_size: entry.map(|e| e.2).unwrap_or(0),
                    work,
                }),
            });
        }
        files.sort_by(|a, b| (!a.tracked, &a.path).cmp(&(!b.tracked, &b.path)));
        Ok(files)
    }

    /// Load exactly one file, checking the scan snapshot before and after reading.
    pub fn load_file(&self, file: &mut File) -> Result<()> {
        if file.loaded {
            return Ok(());
        }
        self.ensure_current(file)?;
        let source = file.source.as_ref().context("Missing file snapshot")?;
        let old = if let Some(oid) = &source.oid {
            if file.old_mode == "120000" {
                self.git(&["cat-file", "blob", oid])?
            } else {
                let mut path_arg = OsString::from("--path=");
                path_arg.push(&file.path);
                self.run(&[
                    OsStr::new("cat-file"),
                    OsStr::new("--filters"),
                    &path_arg,
                    OsStr::new(oid),
                ])?
            }
        } else {
            vec![]
        };
        let current = self.read_work(&file.path)?.unwrap_or_default();
        self.ensure_current(file)?;
        let binary = old.contains(&0)
            || current.contains(&0)
            || std::str::from_utf8(&old).is_err()
            || std::str::from_utf8(&current).is_err()
            || file.mode == "120000"
            || file.old_mode == "120000";
        let mut hunks = if binary {
            vec![]
        } else {
            model::hunks(std::str::from_utf8(&old)?, std::str::from_utf8(&current)?)
        };
        for h in &mut hunks {
            if let Some(previous) = file.hunks.iter().find(|p| p.old == h.old && p.new == h.new) {
                h.selected = previous.selected;
                h.review = previous.review;
            } else {
                h.selected = file.selected;
            }
        }
        file.old = old;
        file.current = current;
        file.hunks = hunks;
        file.binary = binary;
        file.loaded = true;
        Ok(())
    }

    /// A binary preview needs only metadata. Full bytes are read on commit/revert.
    pub fn load_preview(&self, file: &mut File) -> Result<()> {
        if file.loaded {
            return Ok(());
        }
        self.ensure_current(file)?;
        if file.exists && file.mode != "120000" {
            let mut sample = [0; 8192];
            let count = fs::File::open(self.safe_path(&file.path)?)?.read(&mut sample)?;
            if sample[..count].contains(&0) {
                file.binary = true;
            }
        }
        if !file.binary {
            self.load_file(file)?;
            if file.binary {
                file.unload();
            }
        }
        Ok(())
    }

    fn ensure_current(&self, file: &File) -> Result<()> {
        let unchanged = if file.loaded {
            self.read_work(&file.path)?.as_deref() == file.exists.then_some(file.current.as_slice())
        } else if let Some(source) = &file.source {
            work_stamp(&self.safe_path(&file.path)?)? == source.work
        } else {
            false
        };
        ensure!(
            unchanged && work_mode(&self.root.join(&file.path), &file.mode) == file.mode,
            "{} changed outside Difly; reload before continuing",
            file.path.display()
        );
        Ok(())
    }

    pub fn scan(&self) -> Result<Vec<File>> {
        let status = self.git(&[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--no-renames",
        ])?;
        let mut files = Vec::new();
        for record in status.split(|&b| b == 0).filter(|r| r.len() > 3) {
            let code = &record[..2];
            let path = bytes_path(&record[3..]);
            let conflict = code.contains(&b'U') || code == b"AA" || code == b"DD";
            let entry = self.head_entry(&path)?;
            let tracked = code != b"??";
            let base_exists = entry.is_some();
            let (old_mode, old) = entry.unwrap_or_else(|| ("100644".into(), Vec::new()));
            let current = self.read_work(&path)?;
            let exists = current.is_some();
            let current = current.unwrap_or_default();
            let mode = work_mode(&self.root.join(&path), &old_mode);
            let binary = current.contains(&0)
                || old.contains(&0)
                || std::str::from_utf8(&current).is_err()
                || std::str::from_utf8(&old).is_err()
                || mode == "120000"
                || old_mode == "120000";
            let mut hunks = if binary {
                Vec::new()
            } else {
                model::hunks(std::str::from_utf8(&old)?, std::str::from_utf8(&current)?)
            };
            for h in &mut hunks {
                h.selected = tracked && !conflict;
            }
            // Index-only differences may produce no worktree difference. They still need normalization on commit.
            if old == current && exists && mode == old_mode && !conflict && tracked && base_exists {
                continue;
            }
            files.push(File {
                path,
                old,
                current,
                exists,
                base_exists,
                tracked,
                conflict,
                binary,
                mode,
                old_mode,
                hunks,
                selected: tracked && !conflict,
                loaded: true,
                source: None,
            });
        }
        files.sort_by(|a, b| (!a.tracked, &a.path).cmp(&(!b.tracked, &b.path)));
        Ok(files)
    }
    pub fn save(&self, file: &mut File, text: &str) -> Result<()> {
        ensure!(file.loaded, "Load the file before editing");
        ensure!(!file.binary, "Binary files and symlinks cannot be edited");
        ensure!(
            self.read_work(&file.path)? == file.exists.then(|| file.current.clone()),
            "{} changed outside Difly. Reload before saving.",
            file.path.display()
        );
        let full = self.safe_path(&file.path)?;
        if let Some(p) = full.parent() {
            fs::create_dir_all(p)?;
        }
        atomic_write(&full, text.as_bytes())?;
        let was_selected = file.selected;
        file.hunks = model::refresh_hunks(&file.old_text(), text, &file.hunks, was_selected);
        file.current = text.as_bytes().to_vec();
        file.exists = true;
        if let Some(source) = &mut file.source {
            source.work = work_stamp(&full)?;
        }
        file.selected = !file.hunks.is_empty() && file.hunks.iter().all(|h| h.selected);
        Ok(())
    }
    pub fn restore_work(&self, file: &File, bytes: Option<&[u8]>, mode: &str) -> Result<()> {
        ensure!(file.loaded, "Load the file before reverting");
        ensure!(
            self.read_work(&file.path)? == file.exists.then(|| file.current.clone()),
            "File changed outside Difly; reload before reverting"
        );
        let full = self.safe_path(&file.path)?;
        if let Some(bytes) = bytes {
            if let Some(p) = full.parent() {
                fs::create_dir_all(p)?;
            }
            if mode == "120000" {
                #[cfg(unix)]
                {
                    let temp = tempfile::tempdir_in(full.parent().unwrap())?;
                    let link = temp.path().join("link");
                    std::os::unix::fs::symlink(bytes_path(bytes), &link)?;
                    fs::rename(link, &full)?;
                }
                #[cfg(not(unix))]
                {
                    bail!("Reverting symbolic links requires Unix; use Git to restore this link");
                }
            } else {
                atomic_write(&full, bytes)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut permissions = fs::metadata(&full)?.permissions();
                    let bits = permissions.mode();
                    permissions.set_mode(if mode == "100755" {
                        bits | 0o111
                    } else {
                        bits & !0o111
                    });
                    fs::set_permissions(&full, permissions)?;
                }
            }
        } else if fs::symlink_metadata(&full).is_ok() {
            fs::remove_file(full)?;
        }
        Ok(())
    }
    pub fn resolve(&self, file: &File) -> Result<()> {
        ensure!(file.loaded, "Load the file before resolving");
        ensure!(file.conflict, "File is not conflicted");
        ensure!(
            !file
                .text()
                .lines()
                .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "======="),
            "Remove all conflict markers before marking resolved"
        );
        ensure!(
            self.read_work(&file.path)? == file.exists.then(|| file.current.clone()),
            "File changed outside Difly; reload first"
        );
        self.run(&[OsStr::new("add"), OsStr::new("--"), file.path.as_os_str()])?;
        Ok(())
    }
    pub fn conflict_blob(&self, path: &Path, stage: u8) -> Result<Option<Vec<u8>>> {
        let entries = self.run(&[
            OsStr::new("ls-files"),
            OsStr::new("-u"),
            OsStr::new("-z"),
            OsStr::new("--"),
            path.as_os_str(),
        ])?;
        for entry in entries.split(|&b| b == 0).filter(|e| !e.is_empty()) {
            let header = entry.split(|&b| b == b'\t').next().unwrap();
            let fields: Vec<_> = header.split(|&b| b == b' ').collect();
            if fields.get(2).copied() == Some(&[b'0' + stage][..]) {
                return Ok(Some(self.git(&[
                    "cat-file",
                    "blob",
                    std::str::from_utf8(fields[1])?,
                ])?));
            }
        }
        Ok(None)
    }
    pub fn conflict_stage(&self, path: &Path, stage: u8) -> Result<String> {
        Ok(String::from_utf8(
            self.conflict_blob(path, stage)?.unwrap_or_default(),
        )?)
    }
    pub fn commit(
        &self,
        files: &[File],
        message: &str,
        amend: bool,
        expected_head: Option<&str>,
    ) -> Result<String> {
        self.commit_impl(files, message, amend, expected_head, false)
    }
    fn commit_impl(
        &self,
        files: &[File],
        message: &str,
        amend: bool,
        expected_head: Option<&str>,
        prepare_only: bool,
    ) -> Result<String> {
        ensure!(!message.trim().is_empty(), "Enter a commit message");
        for state in [
            "rebase-merge",
            "rebase-apply",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
        ] {
            let path = self.git(&["rev-parse", "--git-path", state])?;
            let path = bytes_path(path.strip_suffix(b"\n").unwrap_or(&path));
            ensure!(
                !self.root.join(path).exists(),
                "Finish the active rebase, cherry-pick, or revert with Git. Difly can edit and resolve files, but only commits ordinary changes and merges."
            );
        }
        ensure!(
            self.head().as_deref() == expected_head,
            "HEAD changed outside Difly; reload before committing"
        );
        ensure!(
            self.git(&["ls-files", "-u"])?.is_empty(),
            "Resolve all conflicts before committing"
        );
        let merge = self.merge();
        ensure!(!(merge && amend), "Cannot amend during a merge");
        ensure!(
            files.iter().all(|f| f.hunks.iter().all(|h| !h.review)),
            "Review changed hunks before committing"
        );
        ensure!(
            amend || merge || files.iter().any(File::included),
            "Select at least one change"
        );
        ensure!(
            !merge
                || files
                    .iter()
                    .all(|f| !f.tracked || f.included() && f.hunks.iter().all(|h| h.selected)),
            "A merge must include all tracked changes"
        );
        for f in files {
            self.ensure_current(f)?;
        }
        if merge {
            let fresh = self.scan_metadata()?;
            ensure!(
                fresh
                    .iter()
                    .filter(|f| f.tracked)
                    .all(|f| files.iter().any(|p| p.path == f.path)),
                "Merge changed outside Difly; reload before committing"
            );
        }
        let temp = tempfile::tempdir()?;
        let index = temp.path().join("index");
        let run = |args: &[&OsStr], input: Option<&[u8]>| -> Result<Vec<u8>> {
            let mut c = self.command();
            c.env("GIT_INDEX_FILE", &index).args(args);
            if input.is_some() {
                c.stdin(Stdio::piped());
            }
            c.stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut child = c.spawn()?;
            if let Some(data) = input {
                child.stdin.take().unwrap().write_all(data)?;
            }
            let output = child.wait_with_output()?;
            ensure!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(output.stdout)
        };
        if expected_head.is_some() {
            run(&[OsStr::new("read-tree"), OsStr::new("HEAD")], None)?;
        } else {
            run(&[OsStr::new("read-tree"), OsStr::new("--empty")], None)?;
        }
        for snapshot in files.iter().filter(|f| f.included()) {
            let mut loaded = snapshot.clone();
            self.load_file(&mut loaded)?;
            let f = &loaded;
            if !f.exists && f.hunks.iter().all(|h| h.selected) {
                run(
                    &[
                        OsStr::new("update-index"),
                        OsStr::new("--force-remove"),
                        OsStr::new("--"),
                        f.path.as_os_str(),
                    ],
                    None,
                )?;
                continue;
            }
            let content = if f.binary || f.hunks.is_empty() {
                f.current.clone()
            } else {
                model::selected_text(&f.old_text(), &f.hunks).into_bytes()
            };
            let mut hash_args = vec![
                OsStr::new("hash-object"),
                OsStr::new("-w"),
                OsStr::new("--stdin"),
            ];
            if f.mode != "120000" {
                hash_args.extend([OsStr::new("--path"), f.path.as_os_str()]);
            }
            let oid = run(&hash_args, Some(&content))?;
            let oid = String::from_utf8(oid)?.trim().to_owned();
            run(
                &[
                    OsStr::new("update-index"),
                    OsStr::new("--add"),
                    OsStr::new("--cacheinfo"),
                    OsStr::new(&f.mode),
                    OsStr::new(&oid),
                    f.path.as_os_str(),
                ],
                None,
            )?;
        }
        ensure!(
            self.head().as_deref() == expected_head,
            "HEAD changed while preparing commit"
        );
        if prepare_only {
            return Ok(String::from_utf8(run(&[OsStr::new("write-tree")], None)?)?
                .trim()
                .into());
        }
        let msg_path = temp.path().join("message");
        fs::write(&msg_path, message)?;
        let mut args = vec![
            OsStr::new("commit"),
            OsStr::new("--file"),
            msg_path.as_os_str(),
        ];
        if amend {
            args.push(OsStr::new("--amend"));
        }
        let output = run(&args, None)?;
        // No working-tree update: leave every unselected edit exactly where it is.
        self.git(&["read-tree", "HEAD"]).context("Commit succeeded, but staging could not be normalized. Run git reset --mixed HEAD to finish")?;
        Ok(String::from_utf8_lossy(&output).trim().into())
    }
}

#[cfg(unix)]
fn bytes_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(bytes))
}
#[cfg(not(unix))]
fn bytes_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).as_ref())
}
#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}
#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}
fn work_mode(path: &Path, fallback: &str) -> String {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return "120000".into();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            return if meta.permissions().mode() & 0o111 != 0 {
                "100755"
            } else {
                "100644"
            }
            .into();
        }
    }
    fallback.into()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing parent directory")?)?;
    if let Ok(meta) = fs::symlink_metadata(path)
        && !meta.file_type().is_symlink()
    {
        temp.as_file().set_permissions(meta.permissions())?;
    }
    #[cfg(unix)]
    if !path.exists() {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn work_stamp(path: &Path) -> Result<WorkStamp> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            Ok(WorkStamp {
                size: meta.len(),
                modified: meta.modified().ok(),
                exists: true,
                #[cfg(unix)]
                identity: (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WorkStamp {
            size: 0,
            modified: None,
            exists: false,
            #[cfg(unix)]
            identity: (0, 0, 0, 0),
        }),
        Err(e) => Err(e.into()),
    }
}
