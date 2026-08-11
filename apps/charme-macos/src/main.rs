#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod bridge;
#[cfg(target_os = "macos")]
mod frame_image;
#[cfg(target_os = "macos")]
mod interaction;
#[cfg(target_os = "macos")]
mod slider;

#[cfg(target_os = "macos")]
fn main() {
    app::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("charme-macos requires macOS");
}
