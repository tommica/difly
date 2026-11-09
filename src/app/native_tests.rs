//! Real GPUI controller coverage without enabling the framework's optional test harness.
//! Every filesystem mutation stays in a disposable repository/config directory.
use super::*;
use std::{fs, process::Command, time::Duration};

pub fn run() {
    let temp = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(temp.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    for (key, value) in [
        ("user.name", "UI Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(&["config", key, value]);
    }
    fs::write(temp.path().join("code.rs"), "one\nkeep\nthree\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "Initial"]);
    fs::write(temp.path().join("code.rs"), "ONE\nkeep\nTHREE\n").unwrap();
    let session = Session::open(temp.path()).unwrap();
    let config = temp.path().join("config/settings.json");
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        super::bind_keys(cx);
        let (handle, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| DiffApp::new(session, difly::settings::Settings::defaults(), config.clone(), window, cx))
        }).unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            handle.update(cx, |_, window, cx| view.update(cx, |this, cx| {
                this.session.select_hunk(1, false).unwrap();
                this.message.update(cx, |s, cx| { s.focus(window, cx); s.replace_text_in_range(None, "Native UI commit", window, cx); });
            })).unwrap();
            cx.background_executor().timer(Duration::from_millis(100)).await;
            handle.update(cx, |_, window, cx| {
                assert!(view.read(cx).can_commit());
                window.dispatch_keystroke(Keystroke::parse(if cfg!(target_os = "macos") { "cmd-enter" } else { "ctrl-enter" }).unwrap(), cx);
            }).unwrap();
            let mut complete = false;
            for _ in 0..50 {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                complete = handle.update(cx, |_, _, cx| !view.read(cx).busy && view.read(cx).status.starts_with("Committed")).unwrap();
                if complete { break; }
            }
            assert!(complete, "Commit never completed");
            handle.update(cx, |_, window, cx| view.update(cx, |this, cx| {
                assert!(this.status.starts_with("Committed"), "{}", this.status);
                assert!(this.message.read(cx).value().is_empty());
                assert_eq!(String::from_utf8(this.session.repo.git(&["show", "HEAD:code.rs"]).unwrap()).unwrap(), "ONE\nkeep\nthree\n");
                assert_eq!(fs::read_to_string(temp.path().join("code.rs")).unwrap(), "ONE\nkeep\nTHREE\n");
                assert!(this.session.repo.git(&["diff", "--cached"]).unwrap().is_empty());
                this.revert(0, window, cx); this.undo_revert(window, cx);
                assert_eq!(fs::read_to_string(temp.path().join("code.rs")).unwrap(), "ONE\nkeep\nTHREE\n");
                this.message.update(cx, |s, cx| s.replace_text_in_range(None, "Draft before amend", window, cx));
            })).unwrap();
            cx.background_executor().timer(Duration::from_millis(100)).await;
            handle.update(cx, |_, window, cx| view.update(cx, |this, cx| {
                this.amend(Some(0), window, cx);
                assert_eq!(this.message.read(cx).value().as_ref(), "Native UI commit");
                this.amend(None, window, cx);
                assert_eq!(this.message.read(cx).value().as_ref(), "Draft before amend");
                this.unified = true; cx.notify();
            })).unwrap();
            cx.background_executor().timer(Duration::from_millis(200)).await;
            handle.update(cx, |_, window, cx| view.update(cx, |this, cx| {
                assert_eq!(this.review.read(cx).files().len(), 1);
                this.unified = false;
                fs::write(temp.path().join("code.rs"), "External edit\n").unwrap();
                this.working.update(cx, |s, cx| s.replace_text_in_range(None, "unsaved draft", window, cx));
                cx.notify();
            })).unwrap();
            cx.background_executor().timer(Duration::from_millis(100)).await;
            handle.update(cx, |_, window, cx| view.update(cx, |this, cx| {
                assert!(this.save_failed); assert!(!this.can_commit());
                assert_eq!(fs::read_to_string(temp.path().join("code.rs")).unwrap(), "External edit\n");
                let settings = difly::settings::Settings::load(&config).unwrap();
                assert_eq!(settings.drafts[&this.session.repo.root], "Draft before amend");
                this.refresh_from_disk(window, cx);
                assert!(!this.save_failed);
                assert_eq!(this.working.read(cx).value().as_ref(), "External edit\n");
            })).unwrap();
            println!("Native UI workflow passed: partial commit, amend draft, revert undo, unified review, external-edit protection and reload.");
            cx.update(|cx| cx.quit());
        }).detach();
    });
}
