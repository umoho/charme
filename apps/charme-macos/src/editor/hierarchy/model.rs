use charme_core::MaterialSlotId;
use charme_renderer::PmxSceneInfo;

use crate::localization::{self, Key};

/// Stable semantic identity used when the native outline selection changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyItemId {
    Scene,
    Model,
    Materials,
    MaterialSlot(MaterialSlotId),
}

pub(super) struct HierarchyNode {
    pub(super) id: HierarchyItemId,
    pub(super) title: String,
    pub(super) children: Vec<usize>,
}

pub(super) struct HierarchySnapshot {
    pub(super) roots: Vec<usize>,
    pub(super) nodes: Vec<HierarchyNode>,
}

impl HierarchySnapshot {
    pub(super) fn empty() -> Self {
        Self {
            roots: vec![0],
            nodes: vec![HierarchyNode {
                id: HierarchyItemId::Scene,
                title: localization::text(Key::Scene).to_owned(),
                children: Vec::new(),
            }],
        }
    }

    pub(super) fn from_scene(info: &PmxSceneInfo) -> Self {
        let mut nodes = vec![
            HierarchyNode {
                id: HierarchyItemId::Scene,
                title: localization::text(Key::Scene).to_owned(),
                children: vec![1],
            },
            HierarchyNode {
                id: HierarchyItemId::Model,
                title: info.name().to_owned(),
                children: vec![2],
            },
            HierarchyNode {
                id: HierarchyItemId::Materials,
                title: localization::text(Key::Materials).to_owned(),
                children: Vec::new(),
            },
        ];

        for slot in info.material_slots() {
            let node_index = nodes.len();
            nodes[2].children.push(node_index);
            let index = format!("{:02}", slot.index());
            nodes.push(HierarchyNode {
                id: HierarchyItemId::MaterialSlot(slot.id()),
                title: localization::format(
                    Key::MaterialSlotListItem,
                    &[("index", &index), ("name", &slot.name())],
                ),
                children: Vec::new(),
            });
        }

        Self {
            roots: vec![0],
            nodes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hierarchy_contains_a_scene_root() {
        let snapshot = HierarchySnapshot::empty();
        assert_eq!(snapshot.roots, vec![0]);
        assert_eq!(snapshot.nodes[0].id, HierarchyItemId::Scene);
        assert!(snapshot.nodes[0].children.is_empty());
    }
}
