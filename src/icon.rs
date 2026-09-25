const PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/difly.png"));

#[cfg(any(not(target_os = "macos"), test))]
pub fn window_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(PNG).expect("valid generated app icon")
}

pub fn with_window_icon(viewport: eframe::egui::ViewportBuilder) -> eframe::egui::ViewportBuilder {
    // AppKit uses the Dock image below; retaining a decoded viewport icon is redundant.
    #[cfg(not(target_os = "macos"))]
    let viewport = viewport.with_icon(window_icon());
    viewport
}

#[cfg(target_os = "macos")]
fn dock_image() -> objc2::rc::Retained<objc2_app_kit::NSImage> {
    use objc2::AnyThread;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSData, NSSize};

    let data = NSData::with_bytes(PNG);
    let image = NSImage::initWithData(NSImage::alloc(), &data).expect("valid app icon");
    // Preserve the 1024px artwork, but avoid oversized native raster caches.
    image.setSize(NSSize::new(128.0, 128.0));
    image
}

// macOS ignores window icons; set the running application's Dock icon explicitly
// so CLI launches don't require a .app bundle.
#[cfg(target_os = "macos")]
pub fn set_application_icon() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let main = MainThreadMarker::new().expect("GUI runs on main thread");
    let image = dock_image();
    // SAFETY: called on the main thread with a valid NSImage. AppKit retains it.
    unsafe { NSApplication::sharedApplication(main).setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
pub fn set_application_icon() {}

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
    #[cfg(target_os = "macos")]
    fn dock_icon_uses_compact_logical_size_without_a_viewport_copy() {
        let image = super::dock_image();
        assert_eq!(image.size(), objc2_foundation::NSSize::new(128.0, 128.0));
        let viewport = super::with_window_icon(eframe::egui::ViewportBuilder::default());
        assert!(viewport.icon.is_none());
    }
}
