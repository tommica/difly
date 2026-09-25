const PNG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/difly.png"));

pub fn window_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(PNG).expect("valid generated app icon")
}

// macOS ignores window icons; set the running application's Dock icon explicitly
// so CLI launches don't require a .app bundle.
#[cfg(target_os = "macos")]
pub fn set_application_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main = MainThreadMarker::new().expect("GUI runs on main thread");
    let data = NSData::with_bytes(PNG);
    let image = NSImage::initWithData(NSImage::alloc(), &data).expect("valid app icon");
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
                .chunks_exact(4)
                .any(|p| p[3] == 255 && p[1] > p[0])
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn appkit_can_decode_the_embedded_icon() {
        use objc2::AnyThread;
        let data = objc2_foundation::NSData::with_bytes(super::PNG);
        let image = objc2_app_kit::NSImage::initWithData(objc2_app_kit::NSImage::alloc(), &data);
        assert!(image.is_some());
    }
}
