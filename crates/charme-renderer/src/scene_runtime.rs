use bevy::prelude::{
    App, Assets, Entity, Handle, Image, Mesh, Mesh3d, MeshMaterial3d, Name, Transform, Vec3,
};
use charme_bevy::{CharmeMaterial, CharmeMaterialParams};
use charme_core::MaterialSlotId;

use crate::{
    overlay::PreviewOverlays,
    pmx_import::{PreparedPmxScene, material_for_record},
    scene::PmxMaterialSlot,
};

pub(crate) struct SpawnedPmxScene {
    entities: Vec<Entity>,
    images: Vec<Handle<Image>>,
    meshes: Vec<Handle<Mesh>>,
    primitive_entities: Vec<Option<Entity>>,
    primitive_component_counts: Vec<usize>,
    overlays: PreviewOverlays,
    pub(crate) materials: Vec<Handle<CharmeMaterial>>,
    default_material_parameters: Vec<CharmeMaterialParams>,
    pub(crate) material_slot_ids: Vec<MaterialSlotId>,
}

impl SpawnedPmxScene {
    pub(crate) fn material_for_slot(
        &self,
        slot_id: MaterialSlotId,
    ) -> Option<&Handle<CharmeMaterial>> {
        self.material_slot_ids
            .iter()
            .position(|candidate| *candidate == slot_id)
            .and_then(|index| self.materials.get(index))
    }

    pub(crate) fn material_state_for_slot(
        &self,
        slot_id: MaterialSlotId,
    ) -> Option<(&Handle<CharmeMaterial>, CharmeMaterialParams)> {
        let index = self
            .material_slot_ids
            .iter()
            .position(|candidate| *candidate == slot_id)?;
        Some((
            self.materials.get(index)?,
            *self.default_material_parameters.get(index)?,
        ))
    }

    pub(crate) fn split_primitives_by_connectivity(
        &mut self,
        app: &mut App,
        primitive_indices: &[usize],
    ) -> bool {
        self.overlays.show_connectivity(
            app,
            &self.primitive_entities,
            &self.primitive_component_counts,
            primitive_indices,
        )
    }
}

impl SpawnedPmxScene {
    pub fn despawn(self, app: &mut App) {
        self.overlays.despawn(app);
        for entity in self.entities {
            let _ = app.world_mut().despawn(entity);
        }
        for handle in self.materials {
            app.world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .remove(handle.id());
        }
        for handle in self.meshes {
            app.world_mut()
                .resource_mut::<Assets<Mesh>>()
                .remove(handle.id());
        }
        for handle in self.images {
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .remove(handle.id());
        }
    }
}

pub(crate) fn spawn_pmx_scene(
    app: &mut App,
    prepared: &PreparedPmxScene,
    mut report: impl FnMut(usize, usize),
) -> SpawnedPmxScene {
    let can_spawn_primitive = prepared.model.primitives().iter().any(|primitive| {
        prepared
            .model
            .material_records()
            .get(primitive.material_index)
            .is_some()
    });
    let total = prepared.textures.len()
        + prepared.model.material_records().len()
        + prepared.model.primitives().len()
        + usize::from(!can_spawn_primitive);
    let mut completed = 0;
    report(completed, total);

    let mut texture_handles = Vec::with_capacity(prepared.textures.len());
    for texture in &prepared.textures {
        texture_handles.push(
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(texture.image.clone()),
        );
        completed += 1;
        report(completed, total);
    }
    let texture_has_alpha = prepared
        .textures
        .iter()
        .map(|texture| texture.has_alpha)
        .collect::<Vec<_>>();
    let center = (prepared.bounds_min + prepared.bounds_max) * 0.5;
    let transform =
        Transform::from_translation(Vec3::new(-center.x, -prepared.bounds_min.y, -center.z));
    let mut entities = Vec::new();
    let mut mesh_handles = Vec::new();
    let mut primitive_entities = vec![None; prepared.model.primitives().len()];
    // Keep one material asset per PMX slot. Apart from avoiding duplicate
    // assets for slots used by multiple primitives, this gives the renderer a
    // stable slot-to-material mapping for material-ball previews.
    let mut material_handles = Vec::with_capacity(prepared.model.material_records().len());
    let mut default_material_parameters =
        Vec::with_capacity(prepared.model.material_records().len());
    for record in prepared.model.material_records() {
        let material = material_for_record(record, &texture_handles, &texture_has_alpha);
        default_material_parameters.push(material.parameters);
        material_handles.push(
            app.world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .add(material),
        );
        completed += 1;
        report(completed, total);
    }

    for (primitive_index, primitive) in prepared.model.primitives().iter().enumerate() {
        if let (Some(record), Some(material)) = (
            prepared
                .model
                .material_records()
                .get(primitive.material_index),
            material_handles.get(primitive.material_index).cloned(),
        ) {
            let mesh = app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(prepared.model.geometry().to_mesh_for_primitive(*primitive));
            mesh_handles.push(mesh.clone());
            let entity = app
                .world_mut()
                .spawn((
                    Name::new(format!(
                        "PMX Primitive {primitive_index} ({})",
                        record.material.name
                    )),
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    transform,
                ))
                .id();
            primitive_entities[primitive_index] = Some(entity);
            entities.push(entity);
        }
        completed += 1;
        report(completed, total);
    }

    if entities.is_empty() {
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(prepared.model.geometry().to_mesh());
        let material = app
            .world_mut()
            .resource_mut::<Assets<CharmeMaterial>>()
            .add(CharmeMaterial::default());
        mesh_handles.push(mesh.clone());
        material_handles.push(material.clone());
        default_material_parameters.push(CharmeMaterialParams::default());
        entities.push(
            app.world_mut()
                .spawn((
                    Name::new("PMX Model"),
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    transform,
                ))
                .id(),
        );
        completed += 1;
        report(completed, total);
    }

    SpawnedPmxScene {
        entities,
        images: texture_handles,
        meshes: mesh_handles,
        primitive_entities,
        primitive_component_counts: prepared
            .primitive_splits
            .iter()
            .map(|split| split.as_ref().map_or(0, |split| split.components.len()))
            .collect(),
        overlays: PreviewOverlays::default(),
        material_slot_ids: prepared
            .info
            .material_slots()
            .iter()
            .map(PmxMaterialSlot::id)
            .collect(),
        materials: material_handles,
        default_material_parameters,
    }
}
