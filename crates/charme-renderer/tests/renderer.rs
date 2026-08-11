use std::{
    thread,
    time::{Duration, Instant},
};

use charme_renderer::{BackgroundColor, OutputSize, PixelFormat, Renderer, RendererConfig};

#[test]
fn renders_and_resizes_an_opaque_cpu_frame() {
    let config = RendererConfig::new(17, 9)
        .pixel_format(PixelFormat::Bgra8Srgb)
        .background(BackgroundColor::rgb(0.8, 0.1, 0.2));
    let mut renderer = Renderer::new(config).expect("renderer should initialize");

    renderer
        .request_redraw()
        .expect("redraw should be accepted");
    let first = wait_for_frame(&mut renderer);

    assert_eq!(first.size(), OutputSize::new(17, 9));
    assert_eq!(first.pixel_format(), PixelFormat::Bgra8Srgb);
    assert_eq!(first.bytes_per_row(), 256);
    assert_eq!(first.pixels().len(), first.bytes_per_row() * 9);
    assert_eq!(first.pixels()[3], 255);
    assert!(first.pixels()[2] > first.pixels()[0]);

    renderer
        .resize(OutputSize::new(0, 0))
        .expect("suspension should be accepted");
    renderer
        .request_redraw()
        .expect("a suspended renderer still accepts redraws");
    thread::sleep(Duration::from_millis(25));
    assert!(
        renderer
            .try_recv_frame()
            .expect("polling should succeed")
            .is_none(),
        "a suspended renderer must not produce frames"
    );

    renderer
        .resize(OutputSize::new(65, 48))
        .expect("resize should be accepted");
    let resized = wait_for_frame(&mut renderer);

    assert!(resized.sequence() > first.sequence());
    assert_eq!(resized.size(), OutputSize::new(65, 48));
    assert_eq!(resized.bytes_per_row(), 512);
    assert_eq!(resized.pixels().len(), resized.bytes_per_row() * 48);

    renderer
        .set_background(BackgroundColor::rgb(0.1, 0.8, 0.2))
        .expect("background update should be accepted");
    let recolored = wait_for_frame(&mut renderer);
    assert!(recolored.sequence() > resized.sequence());
    assert!(recolored.pixels()[1] > recolored.pixels()[2]);

    renderer
        .orbit(0.35, -0.1)
        .expect("orbit input should be accepted");
    let orbited = wait_for_frame(&mut renderer);
    assert!(orbited.sequence() > recolored.sequence());
    assert_ne!(orbited.pixels(), recolored.pixels());

    renderer.zoom(-0.15).expect("zoom input should be accepted");
    let zoomed = wait_for_frame(&mut renderer);
    assert!(zoomed.sequence() > orbited.sequence());
    renderer
        .reset_camera()
        .expect("camera reset should be accepted");
    let reset = wait_for_frame(&mut renderer);
    assert!(reset.sequence() > zoomed.sequence());

    renderer.shutdown().expect("renderer should stop cleanly");
}

fn wait_for_frame(renderer: &mut Renderer) -> charme_renderer::Frame {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match renderer.try_recv_frame() {
            Ok(Some(frame)) => return frame,
            Ok(None) => {}
            Err(error) => panic!("renderer failed: {error}"),
        }
        assert!(Instant::now() < deadline, "renderer timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
