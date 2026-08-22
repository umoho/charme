//! Inspects the connected-component structure of a PMX primitive.
//!
//! Unzips a model, parses the PMX, and for each primitive prints the material
//! name plus a bounding box per connected component. This is used to understand
//! how a primitive (e.g. the skin primitive that contains the legs) is composed.
//!
//! Usage: struct_debug <zip> <pmx_entry> [material_substring]
//!
//! Without the optional third argument, every primitive is analyzed. When a
//! material substring is supplied, only primitives whose material name contains
//! it are reported.

use std::{fs::File, io::Read};

use bevy_pmx::{PmxImportContext, import_pmx, parse_pmx};
use charme_geometry::{PrimitiveRange, split_primitive};

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 2 {
        eprintln!("usage: struct_debug <zip> <pmx_entry> [material_substring]");
        return;
    }
    let filter = args.get(2).cloned();

    let mut archive =
        zip::ZipArchive::new(File::open(&args[0]).expect("open zip")).expect("zip archive");
    let mut entry = archive.by_name(&args[1]).expect("pmx entry");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read pmx");

    let document = parse_pmx(&bytes).expect("parse pmx");
    let model = import_pmx(document, &PmxImportContext::default()).model;
    let geometry = model.geometry();
    let positions = &geometry.positions;
    let indices = &geometry.indices;

    for (prim_index, primitive) in model.primitives().iter().enumerate() {
        let material = model
            .material_records()
            .get(primitive.material_index)
            .map(|record| record.material.name.clone())
            .unwrap_or_default();
        if let Some(filter) = &filter
            && !material.contains(filter.as_str())
        {
            continue;
        }
        let range = PrimitiveRange::new(primitive.index_start, primitive.index_count);
        let split = match split_primitive(indices, positions.len(), range) {
            Ok(split) => split,
            Err(error) => {
                println!("prim {prim_index} mat={material:?} split error: {error}");
                continue;
            }
        };
        println!(
            "prim {prim_index} mat={material:?} components={}",
            split.components.len()
        );
        for (comp_index, component) in split.components.iter().enumerate() {
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for &vertex in &component.vertex_indices {
                let position = positions[vertex as usize];
                for axis in 0..3 {
                    min[axis] = min[axis].min(position[axis]);
                    max[axis] = max[axis].max(position[axis]);
                }
            }
            let extent = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
            println!(
                "  comp {comp_index}: tris={} verts={} bounds x[{:.2},{:.2}] y[{:.2},{:.2}] z[{:.2},{:.2}] extent=({:.2},{:.2},{:.2})",
                component.triangle_count(),
                component.vertex_indices.len(),
                min[0],
                max[0],
                min[1],
                max[1],
                min[2],
                max[2],
                extent[0],
                extent[1],
                extent[2],
            );
        }
    }
}
