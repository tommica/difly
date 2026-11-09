//! Platform icon registration. Linux uses the installed desktop entry's matching app ID.
#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(target_os = "macos")]
pub fn install() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    if let Some(mtm) = MainThreadMarker::new() {
        let data = NSData::with_bytes(include_bytes!(concat!(env!("OUT_DIR"), "/difly.png")));
        if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
            // SAFETY: we are on the main thread and the image is initialized from valid PNG data.
            unsafe {
                NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
            }
        }
    }
}
