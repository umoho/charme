//! Native scene hierarchy built on AppKit's `NSOutlineView`.

mod appkit;
mod model;

use cacao::image::Image;
use charme_core::MaterialSlotId;
use charme_renderer::PmxSceneInfo;

use appkit::NativeHierarchyView;
use model::HierarchySnapshot;

pub(crate) use model::HierarchyItemId;

/// Scene hierarchy panel exposed to the editor window.
pub(crate) struct HierarchyView {
    native: NativeHierarchyView,
}

impl HierarchyView {
    pub(crate) fn new() -> Self {
        Self {
            native: NativeHierarchyView::new(HierarchySnapshot::empty()),
        }
    }

    pub(crate) fn view(&self) -> &cacao::view::View {
        &self.native.view
    }

    pub(crate) fn clear(&self) {
        self.native.set_snapshot(HierarchySnapshot::empty());
    }

    pub(crate) fn set_scene(&self, info: &PmxSceneInfo) {
        self.native
            .set_snapshot(HierarchySnapshot::from_scene(info));
    }

    pub(crate) fn set_material_thumbnail(&self, slot_id: MaterialSlotId, image: &Image) {
        self.native.set_thumbnail(slot_id, image);
    }
}
