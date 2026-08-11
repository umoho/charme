use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use charme_renderer::{
    BackgroundColor, OutputSize, PixelFormat, Renderer, RendererConfig, RendererNotification,
};

#[test]
fn renders_resizes_and_loads_pmx() {
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

    let pmx_path = write_minimal_pmx();
    renderer
        .load_pmx(&pmx_path)
        .expect("PMX load should be accepted");
    let loaded = wait_for_notification(&mut renderer);
    let RendererNotification::PmxLoaded(info) = loaded else {
        panic!("expected a successful PMX notification");
    };
    assert_eq!(info.path(), pmx_path);
    assert_eq!(info.name(), "Charme fixture");
    assert_eq!(info.vertex_count(), 3);
    assert_eq!(info.index_count(), 3);
    assert_eq!(info.material_slots().len(), 1);
    assert_eq!(info.material_slots()[0].name(), "Body");
    assert!(info.warnings().is_empty());
    let pmx_frame = wait_for_frame(&mut renderer);
    assert!(pmx_frame.sequence() > reset.sequence());

    let missing = pmx_path.with_file_name("missing-model.pmx");
    renderer
        .load_pmx(&missing)
        .expect("failed PMX load should still be accepted asynchronously");
    let failed = wait_for_notification(&mut renderer);
    assert!(matches!(
        failed,
        RendererNotification::PmxLoadFailed { path, .. } if path == missing
    ));

    renderer.shutdown().expect("renderer should stop cleanly");
    fs::remove_file(pmx_path).expect("fixture should be removable");
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

fn wait_for_notification(renderer: &mut Renderer) -> RendererNotification {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match renderer.try_recv_notification() {
            Ok(Some(notification)) => return notification,
            Ok(None) => {}
            Err(error) => panic!("renderer failed: {error}"),
        }
        assert!(Instant::now() < deadline, "renderer notification timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

fn write_minimal_pmx() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("charme_renderer_{unique}.pmx"));
    fs::write(&path, minimal_pmx_bytes()).expect("fixture should be writable");
    path
}

fn minimal_pmx_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PMX ");
    push_f32(&mut bytes, 2.0);
    bytes.push(8);
    bytes.push(1); // UTF-8
    bytes.push(0); // additional UV count
    bytes.extend_from_slice(&[4, 4, 4, 4, 4, 4]);
    push_text(&mut bytes, "Charme fixture");
    push_text(&mut bytes, "Charme fixture");
    push_text(&mut bytes, "");
    push_text(&mut bytes, "");

    push_i32(&mut bytes, 3);
    push_vertex(&mut bytes, [-1.0, 0.0, 0.0], [0.0, 0.0]);
    push_vertex(&mut bytes, [1.0, 0.0, 0.0], [1.0, 0.0]);
    push_vertex(&mut bytes, [0.0, 2.0, 0.0], [0.5, 1.0]);
    push_i32(&mut bytes, 3);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, 2);

    push_i32(&mut bytes, 0); // textures
    push_i32(&mut bytes, 1); // materials
    push_text(&mut bytes, "Body");
    push_text(&mut bytes, "Body");
    push_vec4(&mut bytes, [0.8, 0.6, 0.5, 1.0]);
    push_vec3(&mut bytes, [0.0, 0.0, 0.0]);
    push_f32(&mut bytes, 1.0);
    push_vec3(&mut bytes, [0.0, 0.0, 0.0]);
    bytes.push(0); // material flags
    push_vec4(&mut bytes, [0.0, 0.0, 0.0, 0.0]);
    push_f32(&mut bytes, 1.0);
    push_i32(&mut bytes, -1); // diffuse texture
    push_i32(&mut bytes, -1); // sphere texture
    bytes.push(0); // sphere mode
    bytes.push(0); // individual toon texture
    push_i32(&mut bytes, -1);
    push_text(&mut bytes, "");
    push_i32(&mut bytes, 3); // surface index count

    for _ in 0..5 {
        push_i32(&mut bytes, 0); // bones, morphs, frames, rigid bodies, joints
    }
    bytes
}

fn push_vertex(bytes: &mut Vec<u8>, position: [f32; 3], uv: [f32; 2]) {
    push_vec3(bytes, position);
    push_vec3(bytes, [0.0, 0.0, 1.0]);
    push_f32(bytes, uv[0]);
    push_f32(bytes, uv[1]);
    bytes.push(0); // BDEF1
    push_i32(bytes, -1);
    push_f32(bytes, 1.0); // edge scale
}

fn push_text(bytes: &mut Vec<u8>, value: &str) {
    push_i32(bytes, value.len() as i32);
    bytes.extend_from_slice(value.as_bytes());
}

fn push_vec3(bytes: &mut Vec<u8>, value: [f32; 3]) {
    for component in value {
        push_f32(bytes, component);
    }
}

fn push_vec4(bytes: &mut Vec<u8>, value: [f32; 4]) {
    for component in value {
        push_f32(bytes, component);
    }
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
