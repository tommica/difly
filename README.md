# Difly

A standalone Git commit dialog built in Rust with egui. Review and edit diffs, select hunks, commit or amend, and resolve merge conflicts.

![Difly showing an editable side-by-side Git diff](docs/images/difly.png)

## Install and run

With Homebrew (macOS/Linux):

```sh
brew install tommica/tap/difly
difly /path/to/repository        # omit the path for the current repository
```

Update with `brew update && brew upgrade difly`.

Or build from source with Rust/Cargo and Git:

```sh
cargo install --path .
difly
```

Git needs an author configured before committing.

Targets macOS, Linux, and Windows. Tested locally on macOS; CI covers all three.

## Controls

- Use **↑/↓** to navigate the tree, **Space** to toggle inclusion, **←** to collapse/go to the parent, and **→** to expand/go to the next row. **Home/End** jump to the first/last row.
- Toggle **Group by folder** for a tree or flat list of paths.
- Switch between **side-by-side/unified**, **soft wrap**, and **light/dark** themes (theme choice is remembered).
- Drag the center gutter to resize panes; double-click to reset.
- Check hunks to include them. **`>>`** reverts a hunk; **Undo revert** restores it.
- **Ctrl/Cmd+Z** undoes edits; **Ctrl/Cmd+R** redoes. Edits autosave.
- **Amend** lets you choose a recent commit and edit its message. Older commits replay newer history; conflicts leave the original branch unchanged. Linear history only.
- **Ctrl/Cmd+Enter** commits. **Ctrl/Cmd+O** opens another repository.

Tracked changes start checked; unversioned files start unchecked. Editing preserves selection. After committing, remaining changes stay on disk, unstaged. Merge commits require all conflicts resolved and all tracked changes included.

## Demo and development

```sh
bash scripts/create-demo.sh       # requires a new demo-files directory
cargo run -- demo-files
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

Binary files are whole-file changes. Submodules and very large files are outside the current scope. Finish rebases/cherry-picks through Git.

## License

[BSD 3-Clause](LICENSE). Bundled JetBrains Mono uses the [SIL Open Font License](assets/fonts/OFL.txt).
