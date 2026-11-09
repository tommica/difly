//! Opt-in real-window integration test: cargo test --features ui-tests --test ui_workflow
#[allow(dead_code, unused_imports)]
#[path = "../src/app/mod.rs"]
mod app;
fn main() {
    app::native_tests::run();
}
