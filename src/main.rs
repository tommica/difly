mod app;
mod icon;
use difly::{session::Session, settings::Settings};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref().is_some_and(|s| s == "--help" || s == "-h") {
        println!(
            "Difly — review, edit, and commit\n\nUsage: difly [repository-path]\n\nRequires Git on PATH. Changes autosave; commits include checked hunks."
        );
        return Ok(());
    }
    if first
        .as_deref()
        .is_some_and(|s| s == "--version" || s == "-V")
    {
        println!("difly {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    anyhow::ensure!(args.next().is_none(), "Usage: difly [repository-path]");
    let path = first.map(PathBuf::from).unwrap_or(std::env::current_dir()?);
    let session = Session::open(&path)?;
    let settings_path = Settings::path()?;
    let settings = Settings::load_migrating(&settings_path).unwrap_or_else(|e| {
        eprintln!("{e:#}");
        Settings::defaults()
    });
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_app_identity("dev.tommica.difly", "Difly");
            icon::install();
            Theme::change(
                if settings.dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                None,
                cx,
            );
            cx.text_system()
                .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
                    "../assets/fonts/ComicMono.ttf"
                ))])
                .expect("load bundled Comic Mono");
            Theme::global_mut(cx).mono_font_family = "Comic Mono".into();
            app::bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Difly");
                    window.set_app_id("difly");
                    let view = cx
                        .new(|cx| app::DiffApp::new(session, settings, settings_path, window, cx));
                    let weak = view.downgrade();
                    window.on_window_should_close(cx, move |window, cx| {
                        weak.update(cx, |view, cx| view.guard_close(window, cx))
                            .unwrap_or(true)
                    });
                    view
                },
            )
            .expect("open Difly");
            cx.activate(true);
        });
    Ok(())
}
