const PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/difly.png"));

pub fn window_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(PNG).expect("valid generated app icon")
}

pub fn with_window_icon(viewport: eframe::egui::ViewportBuilder) -> eframe::egui::ViewportBuilder {
    // eframe sets both the native window icon and the macOS Dock icon.
    // On Wayland, the app ID must match the installed desktop entry instead.
    viewport.with_app_id("difly").with_icon(window_icon())
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_svg_render_is_a_valid_transparent_icon() {
        let icon = super::window_icon();
        assert_eq!((icon.width, icon.height), (1024, 1024));
        assert_eq!(icon.rgba.len(), 1024 * 1024 * 4);
        assert_eq!(icon.rgba[3], 0);
        assert!(
            icon.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] == 255 && p[1] > p[0])
        );
    }

    #[test]
    fn viewport_has_icon_and_matching_desktop_identity() {
        let viewport = super::with_window_icon(eframe::egui::ViewportBuilder::default());
        assert!(viewport.icon.is_some());
        assert_eq!(viewport.app_id.as_deref(), Some("difly"));
    }
}
