fn main() {
    println!("cargo:rerun-if-changed=assets/icons/difly.svg");
    let svg = std::fs::read("assets/icons/difly.svg").expect("read app icon");
    let tree = resvg::usvg::Tree::from_data(&svg, &Default::default()).expect("parse app icon");
    let size = tree.size();
    let scale = 1024.0 / size.width().max(size.height());
    let mut pixmap = resvg::tiny_skia::Pixmap::new(1024, 1024).unwrap();
    let transform = resvg::tiny_skia::Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        (1024.0 - size.width() * scale) / 2.0,
        (1024.0 - size.height() * scale) / 2.0,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    pixmap
        .save_png(output.join("difly.png"))
        .expect("write app icon");
}
