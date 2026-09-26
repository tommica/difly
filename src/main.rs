mod diff_navigation;
mod diff_view;
mod file_tree;
mod icon;
mod ui;
mod virtual_diff;
use std::path::PathBuf;
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    if path == std::path::Path::new("--help") || path == std::path::Path::new("-h") {
        println!(
            "Difly — review, edit, and commit\n\nUsage: difly [repository-path]\n\nOpens the repository containing the path (default: current directory).\nRequires Git on PATH. Changes autosave; commits include checked hunks."
        );
        return Ok(());
    }
    if path == std::path::Path::new("--version") {
        println!("difly {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Difly")
        .with_inner_size([1400.0, 900.0])
        .with_min_inner_size([900.0, 600.0]);
    let options = eframe::NativeOptions {
        viewport: icon::with_window_icon(viewport),
        ..Default::default()
    };
    eframe::run_native(
        "Difly",
        options,
        Box::new(move |cc| {
            icon::set_application_icon();
            Ok(Box::new(ui::App::new(cc, path)))
        }),
    )
}
