#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod bridge;
#[cfg(all(target_os = "macos", feature = "debug-ui"))]
mod debug;
#[cfg(target_os = "macos")]
mod docking;
#[cfg(target_os = "macos")]
mod frame_image;
#[cfg(target_os = "macos")]
mod interaction;
#[cfg(target_os = "macos")]
mod localization;
#[cfg(target_os = "macos")]
mod parameter_control;
#[cfg(target_os = "macos")]
mod shader_inspection;
#[cfg(target_os = "macos")]
mod slider;

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
