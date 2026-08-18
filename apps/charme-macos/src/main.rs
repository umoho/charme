#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

#[cfg(target_os = "macos")]
mod app;
#[cfg(all(target_os = "macos", feature = "debug-ui"))]
mod debug;
#[cfg(target_os = "macos")]
mod editor;
#[cfg(target_os = "macos")]
mod loading;
#[cfg(target_os = "macos")]
mod localization;
#[cfg(target_os = "macos")]
mod preview;
#[cfg(target_os = "macos")]
mod shader_inspection;
#[cfg(target_os = "macos")]
mod startup;
#[cfg(target_os = "macos")]
mod ui;

#[cfg(target_os = "macos")]
fn main() {
    #[cfg(feature = "debug-ui")]
    app::run_with_debug_state(debug::state_from_args());
    #[cfg(not(feature = "debug-ui"))]
    app::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("charme-macos requires macOS");
}
