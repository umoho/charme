use std::{
    fs,
    io::Write,
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use charme_core::ParameterValue;
use charme_renderer::{
    BackgroundColor, OutputSize, PixelFormat, PmxLoadRequest, PmxLoadStage, Renderer,
    RendererConfig, RendererNotification,
};
use zip::{ZipWriter, write::SimpleFileOptions};

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
        .set_material_parameter("material.roughness", ParameterValue::F32(1.0))
        .expect("material parameter should be accepted");
    let tinted = wait_for_frame(&mut renderer);
    assert!(tinted.sequence() > first.sequence());
    assert_ne!(tinted.pixels(), first.pixels());

    renderer
        .set_material_parameter("material.not_a_fixed_parameter", ParameterValue::F32(1.0))
        .expect("invalid material parameters are reported asynchronously");
    assert!(matches!(
        wait_for_notification(&mut renderer),
        RendererNotification::MaterialParameterRejected { path, .. }
            if path == "material.not_a_fixed_parameter"
    ));

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
        .load_pmx_request(
            PmxLoadRequest::from_path(pmx_path.clone(), None, Vec::new()).with_request_id(42),
        )
        .expect("PMX load should be accepted");
    let mut progress_events = Vec::new();
    let (load_request_id, info) = loop {
        match wait_for_notification(&mut renderer) {
            RendererNotification::PmxLoadProgress(progress) => progress_events.push(progress),
            RendererNotification::PmxLoaded { request_id, info } => break (request_id, info),
            notification => panic!("expected PMX progress or completion, got {notification:?}"),
        }
    };
    assert!(!progress_events.is_empty());
    assert_eq!(load_request_id, 42);
    assert!(
        progress_events
            .iter()
            .all(|progress| progress.request_id() == load_request_id)
    );
    let stages = progress_events
        .iter()
        .map(|progress| progress.stage())
        .collect::<Vec<_>>();
    assert!(stages.contains(&PmxLoadStage::ReadingPmx));
    assert!(stages.contains(&PmxLoadStage::ParsingPmx));
    assert!(stages.contains(&PmxLoadStage::LoadingTextures));
    assert!(stages.contains(&PmxLoadStage::BuildingSelection));
    assert!(stages.contains(&PmxLoadStage::BuildingScene));
    let texture_progress = progress_events
        .iter()
        .find(|progress| progress.stage() == PmxLoadStage::LoadingTextures)
        .expect("texture stage should be reported");
    assert_eq!(texture_progress.completed(), Some(0));
    assert_eq!(texture_progress.total(), Some(0));
    assert_eq!(texture_progress.fraction(), None);
    assert_eq!(info.path(), pmx_path);
    assert_eq!(info.archive_entry(), None);
    assert_eq!(info.name(), "Charme fixture");
    assert_eq!(info.vertex_count(), 3);
    assert_eq!(info.index_count(), 3);
    assert_eq!(info.material_slots().len(), 1);
    assert_eq!(info.material_slots()[0].name(), "Body");
    assert_eq!(info.primitives().len(), 1);
    assert_eq!(info.primitives()[0].components().len(), 1);
    assert_eq!(info.primitives()[0].components()[0].triangle_count(), 1);
    let slot_id = info.material_slots()[0].id();
    assert!(info.warnings().is_empty());

    renderer
        .pick_viewport(32.5, 24.0)
        .expect("viewport picking should be accepted");
    let picked = wait_for_notification(&mut renderer);
    assert!(matches!(
        picked,
        RendererNotification::ViewportPickResult {
            slot_id: Some(picked_slot),
            primitive_index: Some(0),
            ..
        } if picked_slot == slot_id
    ));

    renderer
        .set_material_parameter_for_slot(slot_id, "material.roughness", ParameterValue::F32(0.2))
        .expect("targeted material parameter should be accepted");
    let pmx_frame = wait_for_frame(&mut renderer);

    renderer
        .set_selected_material_slot(Some(slot_id))
        .expect("material selection should be accepted");
    let outlined = wait_for_frame(&mut renderer);
    assert_ne!(outlined.pixels(), pmx_frame.pixels());
    assert!(pmx_frame.sequence() > reset.sequence());
    let thumbnail = wait_for_material_thumbnail(&mut renderer);
    let RendererNotification::MaterialThumbnailReady {
        source,
        slot_index,
        frame,
        ..
    } = thumbnail
    else {
        panic!("expected a material thumbnail notification");
    };
    assert_eq!(source.path(), pmx_path.as_path());
    assert_eq!(source.archive_entry(), None);
    assert_eq!(slot_index, 0);
    assert_eq!(frame.size(), OutputSize::new(64, 64));
    assert_eq!(frame.pixel_format(), PixelFormat::Bgra8Srgb);

    renderer
        .request_material_inspector_preview(0)
        .expect("inspector preview should be accepted");
    let inspector_preview = wait_for_notification(&mut renderer);
    let RendererNotification::MaterialInspectorPreviewReady {
        source,
        slot_index,
        frame,
        ..
    } = inspector_preview
    else {
        panic!("expected an inspector material preview notification");
    };
    assert_eq!(source.path(), pmx_path.as_path());
    assert_eq!(source.archive_entry(), None);
    assert_eq!(slot_index, 0);
    assert_eq!(frame.size(), OutputSize::new(256, 256));
    assert_eq!(frame.pixel_format(), PixelFormat::Bgra8Srgb);
    assert_no_notification(&mut renderer, Duration::from_millis(100));

    renderer
        .clear_pmx()
        .expect("clearing the PMX scene should be accepted");
    let cleared = wait_for_frame(&mut renderer);
    assert!(cleared.sequence() > outlined.sequence());

    renderer
        .load_pmx(&pmx_path)
        .expect("reloading PMX after clearing should be accepted");
    let (_, reloaded) = wait_for_loaded(&mut renderer);
    assert_eq!(reloaded.path(), pmx_path);

    let missing = pmx_path.with_file_name("missing-model.pmx");
    renderer
        .load_pmx(&missing)
        .expect("failed PMX load should still be accepted asynchronously");
    let failed = wait_for_failed(&mut renderer);
    assert!(matches!(
        failed,
        RendererNotification::PmxLoadFailed { source, .. }
            if source.path() == missing.as_path()
    ));

    renderer
        .pick_viewport(32.5, 24.0)
        .expect("picking should still query the previous scene after failure");
    assert!(matches!(
        wait_for_notification(&mut renderer),
        RendererNotification::ViewportPickResult { source, .. }
            if source.path() == pmx_path.as_path() && source.archive_entry().is_none()
    ));

    let zip_path = write_minimal_pmx_zip();
    renderer
        .load_pmx_with_source(
            &zip_path,
            Some("Model/character.pmx".to_owned()),
            Vec::new(),
        )
        .expect("ZIP PMX load should be accepted");
    let (_, zip_info) = wait_for_loaded(&mut renderer);
    assert_eq!(zip_info.path(), zip_path);
    assert_eq!(zip_info.archive_entry(), Some("Model/character.pmx"));
    assert_eq!(zip_info.name(), "Charme fixture");

    renderer.shutdown().expect("renderer should stop cleanly");
    fs::remove_file(pmx_path).expect("fixture should be removable");
    fs::remove_file(zip_path).expect("ZIP fixture should be removable");
}

#[test]
fn temporary_split_keeps_camera_updates() {
    let config = RendererConfig::new(64, 48).pixel_format(PixelFormat::Bgra8Srgb);
    let mut renderer = Renderer::new(config).expect("renderer should initialize");
    let path = write_disconnected_pmx();

    renderer
        .load_pmx_request(
            PmxLoadRequest::from_path(path.clone(), None, Vec::new()).with_request_id(7),
        )
        .expect("PMX load should be accepted");
    let (_, info) = wait_for_loaded(&mut renderer);
    assert_eq!(info.primitives()[0].components().len(), 2);

    renderer
        .set_selected_primitives(vec![0])
        .expect("primitive selection should be accepted");
    let selected = wait_for_frame(&mut renderer);
    renderer
        .split_selected_primitives_by_connectivity(vec![0])
        .expect("temporary primitive split should be accepted");
    let split = wait_for_frame(&mut renderer);
    assert!(split.sequence() > selected.sequence());

    renderer
        .orbit(0.35, -0.1)
        .expect("orbit input should be accepted after splitting");
    let orbited = wait_for_frame(&mut renderer);
    assert!(orbited.sequence() > split.sequence());
    assert_ne!(orbited.pixels(), split.pixels());

    renderer.shutdown().expect("renderer should stop cleanly");
    fs::remove_file(path).expect("fixture should be removable");
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

fn wait_for_material_thumbnail(renderer: &mut Renderer) -> RendererNotification {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match renderer.try_recv_material_thumbnail() {
            Ok(Some(notification)) => return notification,
            Ok(None) => {}
            Err(error) => panic!("renderer failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "material thumbnail notification timed out"
        );
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

fn wait_for_loaded(renderer: &mut Renderer) -> (u64, charme_renderer::PmxSceneInfo) {
    loop {
        match wait_for_notification(renderer) {
            RendererNotification::PmxLoadProgress(_) => {}
            RendererNotification::PmxLoaded { request_id, info } => return (request_id, info),
            notification => panic!("expected PMX completion, got {notification:?}"),
        }
    }
}

fn wait_for_failed(renderer: &mut Renderer) -> RendererNotification {
    loop {
        match wait_for_notification(renderer) {
            RendererNotification::PmxLoadProgress(_) => {}
            notification @ RendererNotification::PmxLoadFailed { .. } => return notification,
            notification => panic!("expected PMX failure, got {notification:?}"),
        }
    }
}

fn assert_no_notification(renderer: &mut Renderer, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        match renderer.try_recv_notification() {
            Ok(Some(notification)) => panic!("unexpected renderer notification: {notification:?}"),
            Ok(None) => {}
            Err(error) => panic!("renderer failed: {error}"),
        }
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

fn write_disconnected_pmx() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("charme_renderer_disconnected_{unique}.pmx"));
    fs::write(&path, disconnected_pmx_bytes()).expect("fixture should be writable");
    path
}

fn write_minimal_pmx_zip() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("charme_renderer_{unique}.zip"));
    let file = fs::File::create(&path).expect("ZIP fixture should be writable");
    let mut writer = ZipWriter::new(file);
    writer
        .start_file("Model/character.pmx", SimpleFileOptions::default())
        .expect("ZIP PMX entry should be created");
    writer
        .write_all(&minimal_pmx_bytes())
        .expect("ZIP PMX entry should be written");
    writer.finish().expect("ZIP fixture should be finished");
    path
}

fn disconnected_pmx_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PMX ");
    push_f32(&mut bytes, 2.0);
    bytes.push(8);
    bytes.push(1); // UTF-8
    bytes.push(0); // additional UV count
    bytes.extend_from_slice(&[4, 4, 4, 4, 4, 4]);
    push_text(&mut bytes, "Charme disconnected fixture");
    push_text(&mut bytes, "Charme disconnected fixture");
    push_text(&mut bytes, "");
    push_text(&mut bytes, "");

    push_i32(&mut bytes, 6);
    push_vertex(&mut bytes, [-1.0, 0.0, 0.0], [0.0, 0.0]);
    push_vertex(&mut bytes, [1.0, 0.0, 0.0], [1.0, 0.0]);
    push_vertex(&mut bytes, [0.0, 2.0, 0.0], [0.5, 1.0]);
    push_vertex(&mut bytes, [3.0, 0.0, 0.0], [0.0, 0.0]);
    push_vertex(&mut bytes, [5.0, 0.0, 0.0], [1.0, 0.0]);
    push_vertex(&mut bytes, [4.0, 2.0, 0.0], [0.5, 1.0]);
    push_i32(&mut bytes, 6);
    for index in 0..6 {
        push_u32(&mut bytes, index);
    }

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
    push_i32(&mut bytes, 6); // surface index count

    for _ in 0..5 {
        push_i32(&mut bytes, 0); // bones, morphs, frames, rigid bodies, joints
    }
    bytes
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
