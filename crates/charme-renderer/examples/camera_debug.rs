//! Offline camera-debug renderer.
//!
//! Loads a PMX model (optionally inside a ZIP archive), then renders a set of
//! (camera orbit × selection) combinations described by a config file, writing
//! each frame as a PNG. This is a debugging aid for the selection-outline and
//! viewport framing; it is not part of the shipped renderer.
//!
//! Usage:
//!   camera_debug <zip> <archive_entry> <config> [--cam <dump>] [--target x,y,z]
//!
//!   <zip>            Path to a PMX file or a ZIP containing one.
//!   <archive_entry>  The PMX entry inside the ZIP (empty for a bare file).
//!   <config>         A text file of render requests, one per line:
//!                      <dx> <dy> <dz> <prim_csv|ALL> <out_name>
//!                    where dx/dy are yaw/pitch offsets in radians, dz is a
//!                    logarithmic zoom offset, and out_name is the PNG stem.
//!   --cam <dump>     Read a camera-state dump (yaw pitch dist tx ty tz) from a
//!                    file produced by the renderer under CHARME_CAMERA_DUMP and
//!                    use it as the base camera instead of `reset_camera`.
//!   --target x,y,z   Override the camera target (focus point).
//!
//! The output size defaults to 512x768; set CAM_W/CAM_H to change it.

use std::{
    env, fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use charme_renderer::{
    BackgroundColor, Frame, PixelFormat, Renderer, RendererConfig, RendererNotification,
};
use image::ExtendedColorType;

const CAMERA_TIMEOUT: Duration = Duration::from_secs(30);

fn wait_for_notification(renderer: &mut Renderer) -> RendererNotification {
    let deadline = Instant::now() + CAMERA_TIMEOUT;
    loop {
        match renderer.try_recv_notification() {
            Ok(Some(notification)) => return notification,
            Ok(None) => {}
            Err(error) => panic!("renderer failed: {error}"),
        }
        assert!(Instant::now() < deadline, "notification timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

fn wait_for_loaded(renderer: &mut Renderer) -> charme_renderer::PmxSceneInfo {
    loop {
        match wait_for_notification(renderer) {
            RendererNotification::PmxLoadProgress(_) => {}
            RendererNotification::PmxLoaded { info, .. } => return info,
            notification => panic!("expected PMX completion, got {notification:?}"),
        }
    }
}

fn wait_for_frame(renderer: &mut Renderer) -> Frame {
    let deadline = Instant::now() + CAMERA_TIMEOUT;
    loop {
        match renderer.try_recv_frame() {
            Ok(Some(frame)) => return frame,
            Ok(None) => {}
            Err(error) => panic!("renderer failed: {error}"),
        }
        assert!(Instant::now() < deadline, "frame timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Parses a camera-state dump line: `yaw pitch distance tx ty tz`.
fn read_cam_dump(path: &str) -> Option<[f32; 6]> {
    let text = fs::read_to_string(path).ok()?;
    let values = text
        .split_whitespace()
        .take(6)
        .map(|token| token.parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()?;
    if values.len() < 6 {
        return None;
    }
    let mut array = [0.0_f32; 6];
    array.copy_from_slice(&values[..6]);
    Some(array)
}

fn cam_dump_string(cam: [f32; 6]) -> String {
    format!(
        "yaw={} pitch={} dist={} target=({},{},{})",
        cam[0], cam[1], cam[2], cam[3], cam[4], cam[5]
    )
}

/// Writes a frame as a PNG, converting BGRA/RGBA to tightly-packed RGBA.
fn write_frame_png(name: &str, frame: &Frame) {
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let bytes_per_row = frame.bytes_per_row();
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let row = &frame.pixels()[y * bytes_per_row..y * bytes_per_row + width * 4];
        for pixel in row.chunks_exact(4) {
            match frame.pixel_format() {
                PixelFormat::Bgra8Srgb => {
                    let [b, g, r, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                    rgba.extend_from_slice(&[r, g, b, a]);
                }
                PixelFormat::Rgba8Srgb => rgba.extend_from_slice(pixel),
                _ => {}
            }
        }
    }
    let path = format!("{name}.png");
    image::save_buffer(
        &path,
        &rgba,
        width as u32,
        height as u32,
        ExtendedColorType::Rgba8,
    )
    .expect("write png");
}

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();

    let mut cam_file: Option<String> = None;
    let mut target_override: Option<[f32; 3]> = None;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cam" => {
                cam_file = args.get(i + 1).cloned();
                i += 2;
            }
            "--target" => {
                if let Some(value) = args.get(i + 1)
                    && let Some(parts) = value
                        .split(',')
                        .map(|token| token.parse::<f32>().ok())
                        .collect::<Option<Vec<_>>>()
                    && parts.len() == 3
                {
                    target_override = Some([parts[0], parts[1], parts[2]]);
                }
                i += 2;
            }
            _ => {
                positional.push(args[i].clone());
                i += 1;
            }
        }
    }

    if positional.len() < 3 {
        eprintln!(
            "usage: camera_debug <zip> <archive_entry> <config> [--cam <dump>] [--target x,y,z]"
        );
        return;
    }
    let zip = &positional[0];
    let entry = &positional[1];
    let config_path = PathBuf::from(&positional[2]);

    let mut cam = cam_file.as_deref().and_then(read_cam_dump);
    if let Some(target) = target_override {
        if let Some(cam) = cam.as_mut() {
            cam[3] = target[0];
            cam[4] = target[1];
            cam[5] = target[2];
            println!("target overridden: {}", cam_dump_string(*cam));
        }
    } else if let Some(cam) = cam.as_ref() {
        println!("using dumped camera: {}", cam_dump_string(*cam));
    }

    let width = env::var("CAM_W")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(512);
    let height = env::var("CAM_H")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(768);
    let config = RendererConfig::new(width, height)
        .pixel_format(PixelFormat::Bgra8Srgb)
        .background(BackgroundColor::rgb(0.35, 0.35, 0.38));

    let mut renderer = Renderer::new(config).expect("renderer init");
    renderer
        .load_pmx_with_source(zip, Some(entry.clone()), Vec::new())
        .expect("load pmx");
    let info = wait_for_loaded(&mut renderer);

    println!(
        "model: {} verts, {} idx, {} prims, {} slots",
        info.vertex_count(),
        info.index_count(),
        info.primitives().len(),
        info.material_slots().len()
    );
    for primitive in info.primitives() {
        let slot = primitive
            .material_slot_id()
            .and_then(|id| info.material_slots().iter().find(|slot| slot.id() == id));
        let material = slot.map(|slot| slot.name().to_string()).unwrap_or_default();
        let triangles = primitive
            .components()
            .iter()
            .map(|component| component.triangle_count())
            .collect::<Vec<_>>();
        println!(
            "prim {:3} mat={material:?} tris={triangles:?}",
            primitive.index()
        );
    }

    let text = fs::read_to_string(&config_path).expect("read config file");
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5 {
            eprintln!("skip malformed config line: {line}");
            continue;
        }
        let dx = fields[0].parse::<f32>().expect("dx");
        let dy = fields[1].parse::<f32>().expect("dy");
        let dz = fields[2].parse::<f32>().expect("dz");
        let prims = if fields[3] == "ALL" {
            info.primitives()
                .iter()
                .map(|primitive| primitive.index())
                .collect::<Vec<_>>()
        } else {
            fields[3]
                .split(',')
                .map(|part| part.parse::<usize>().expect("prim index"))
                .collect::<Vec<_>>()
        };
        let out = fields[4];

        if let Some(cam) = cam {
            renderer
                .set_camera_absolute(cam[0], cam[1], cam[2], [cam[3], cam[4], cam[5]])
                .expect("set absolute camera");
            renderer.orbit(dx, dy).expect("orbit offset");
            renderer.zoom(dz).expect("zoom offset");
        } else {
            renderer.reset_camera().expect("reset camera");
            renderer.orbit(dx, dy).expect("orbit");
            renderer.zoom(dz).expect("zoom");
        }
        renderer.set_selected_primitives(prims).expect("select");

        // Warm-up frame: the selection outline and texture upload may need one
        // frame before the composite node includes them fully.
        renderer.request_redraw().expect("redraw");
        let _ = wait_for_frame(&mut renderer);
        renderer.request_redraw().expect("redraw");
        let frame = wait_for_frame(&mut renderer);
        write_frame_png(out, &frame);
        println!("wrote {out}.png");
    }

    renderer.shutdown().expect("shutdown");
}
