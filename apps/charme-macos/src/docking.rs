//! UI-independent docking layout model and geometry calculation.
//!
//! Coordinates use a top-left origin: horizontal splits run left-to-right,
//! while vertical splits run top-to-bottom. A Cacao/AppKit adapter can convert
//! these rectangles if its view uses a different coordinate system.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

/// The smallest ratio accepted for either side of a split.
pub const MIN_SPLIT_RATIO: f64 = 0.05;
/// The largest ratio accepted for the first side of a split.
pub const MAX_SPLIT_RATIO: f64 = 1.0 - MIN_SPLIT_RATIO;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PanelId(String);

impl PanelId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PanelId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PanelId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u64);

impl NodeId {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Children are arranged left-to-right.
    Horizontal,
    /// Children are arranged top-to-bottom.
    Vertical,
}

/// A finite split ratio kept inside the supported range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitRatio(f64);

impl SplitRatio {
    pub const DEFAULT: Self = Self(0.5);

    pub fn new(value: f64) -> Self {
        if value.is_finite() {
            Self(value.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO))
        } else {
            Self::DEFAULT
        }
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl Default for SplitRatio {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DockNode {
    Split {
        axis: Axis,
        ratio: SplitRatio,
        first: NodeId,
        second: NodeId,
    },
    Tabs {
        panels: Vec<PanelId>,
        active: PanelId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DockTree {
    root: NodeId,
    nodes: BTreeMap<NodeId, DockNode>,
}

impl DockTree {
    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn node(&self, id: NodeId) -> Option<&DockNode> {
        self.nodes.get(&id)
    }

    pub fn set_split_ratio(&mut self, id: NodeId, value: f64) -> Result<SplitRatio, DockError> {
        let node = self.nodes.get_mut(&id).ok_or(DockError::UnknownNode(id))?;
        let DockNode::Split { ratio, .. } = node else {
            return Err(DockError::NotSplitNode(id));
        };

        *ratio = SplitRatio::new(value);
        Ok(*ratio)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Builds a valid tree from leaves toward the root.
///
/// Nodes may be assembled in any order allowed by their references. `build`
/// rejects shared children, cycles, and nodes that are not reachable from the
/// selected root.
#[derive(Clone, Debug, Default)]
pub struct DockTreeBuilder {
    next_id: u64,
    nodes: BTreeMap<NodeId, DockNode>,
}

impl DockTreeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tabs<I>(&mut self, panels: I, active: impl Into<PanelId>) -> Result<NodeId, DockError>
    where
        I: IntoIterator<Item = PanelId>,
    {
        let panels: Vec<_> = panels.into_iter().collect();
        if panels.is_empty() {
            return Err(DockError::EmptyTabs);
        }

        let active = active.into();
        if !panels.contains(&active) {
            return Err(DockError::ActivePanelMissing(active));
        }

        Ok(self.insert(DockNode::Tabs { panels, active }))
    }

    pub fn split(
        &mut self,
        axis: Axis,
        ratio: f64,
        first: NodeId,
        second: NodeId,
    ) -> Result<NodeId, DockError> {
        if first == second {
            return Err(DockError::DuplicateChild(first));
        }
        if !self.nodes.contains_key(&first) {
            return Err(DockError::UnknownNode(first));
        }
        if !self.nodes.contains_key(&second) {
            return Err(DockError::UnknownNode(second));
        }

        Ok(self.insert(DockNode::Split {
            axis,
            ratio: SplitRatio::new(ratio),
            first,
            second,
        }))
    }

    pub fn build(self, root: NodeId) -> Result<DockTree, DockError> {
        if !self.nodes.contains_key(&root) {
            return Err(DockError::UnknownNode(root));
        }

        validate_tree(root, &self.nodes)?;
        Ok(DockTree {
            root,
            nodes: self.nodes,
        })
    }

    fn insert(&mut self, node: DockNode) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("docking node id space exhausted");
        self.nodes.insert(id, node);
        id
    }
}

fn validate_tree(root: NodeId, nodes: &BTreeMap<NodeId, DockNode>) -> Result<(), DockError> {
    fn visit(
        id: NodeId,
        nodes: &BTreeMap<NodeId, DockNode>,
        visiting: &mut BTreeSet<NodeId>,
        visited: &mut BTreeSet<NodeId>,
    ) -> Result<(), DockError> {
        if visiting.contains(&id) {
            return Err(DockError::Cycle(id));
        }
        if visited.contains(&id) {
            return Err(DockError::SharedNode(id));
        }

        let node = nodes.get(&id).ok_or(DockError::UnknownNode(id))?;
        visiting.insert(id);

        if let DockNode::Split { first, second, .. } = node {
            visit(*first, nodes, visiting, visited)?;
            visit(*second, nodes, visiting, visited)?;
        }

        visiting.remove(&id);
        visited.insert(id);
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    visit(root, nodes, &mut visiting, &mut visited)?;

    if visited.len() != nodes.len() {
        let id = nodes
            .keys()
            .find(|id| !visited.contains(id))
            .copied()
            .expect("node counts differ, so an unreachable node must exist");
        return Err(DockError::UnreachableNode(id));
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    pub divider_thickness: f64,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            divider_thickness: 6.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneGeometry {
    pub node: NodeId,
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DividerGeometry {
    /// The split node that owns this divider.
    pub node: NodeId,
    pub axis: Axis,
    /// The complete rectangle assigned to the owning split.
    pub split_rect: Rect,
    pub rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockGeometry {
    pub panes: Vec<PaneGeometry>,
    pub dividers: Vec<DividerGeometry>,
}

/// Computes pane and divider rectangles in deterministic, depth-first order.
pub fn compute_geometry(
    tree: &DockTree,
    bounds: Rect,
    options: LayoutOptions,
) -> Result<DockGeometry, LayoutError> {
    if !bounds.is_valid() {
        return Err(LayoutError::InvalidBounds(bounds));
    }
    if !options.divider_thickness.is_finite() || options.divider_thickness < 0.0 {
        return Err(LayoutError::InvalidDividerThickness(
            options.divider_thickness,
        ));
    }

    fn visit(
        tree: &DockTree,
        id: NodeId,
        rect: Rect,
        divider_thickness: f64,
        output: &mut DockGeometry,
    ) -> Result<(), LayoutError> {
        match tree.node(id).ok_or(LayoutError::UnknownNode(id))? {
            DockNode::Tabs { .. } => output.panes.push(PaneGeometry { node: id, rect }),
            DockNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let extent = match axis {
                    Axis::Horizontal => rect.width,
                    Axis::Vertical => rect.height,
                };
                let divider_extent = divider_thickness.min(extent);
                let available = extent - divider_extent;
                let first_extent = available * ratio.get();
                let second_extent = available - first_extent;

                let (first_rect, divider_rect, second_rect) = match axis {
                    Axis::Horizontal => (
                        Rect::new(rect.x, rect.y, first_extent, rect.height),
                        Rect::new(rect.x + first_extent, rect.y, divider_extent, rect.height),
                        Rect::new(
                            rect.x + first_extent + divider_extent,
                            rect.y,
                            second_extent,
                            rect.height,
                        ),
                    ),
                    Axis::Vertical => (
                        Rect::new(rect.x, rect.y, rect.width, first_extent),
                        Rect::new(rect.x, rect.y + first_extent, rect.width, divider_extent),
                        Rect::new(
                            rect.x,
                            rect.y + first_extent + divider_extent,
                            rect.width,
                            second_extent,
                        ),
                    ),
                };

                output.dividers.push(DividerGeometry {
                    node: id,
                    axis: *axis,
                    split_rect: rect,
                    rect: divider_rect,
                });
                visit(tree, *first, first_rect, divider_thickness, output)?;
                visit(tree, *second, second_rect, divider_thickness, output)?;
            }
        }

        Ok(())
    }

    let mut output = DockGeometry::default();
    visit(
        tree,
        tree.root(),
        bounds,
        options.divider_thickness,
        &mut output,
    )?;
    Ok(output)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockError {
    EmptyTabs,
    ActivePanelMissing(PanelId),
    UnknownNode(NodeId),
    NotSplitNode(NodeId),
    DuplicateChild(NodeId),
    SharedNode(NodeId),
    Cycle(NodeId),
    UnreachableNode(NodeId),
}

impl fmt::Display for DockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTabs => write!(formatter, "a tab node must contain at least one panel"),
            Self::ActivePanelMissing(panel) => write!(
                formatter,
                "active panel '{}' is not present in its tab node",
                panel.as_str()
            ),
            Self::UnknownNode(node) => write!(formatter, "unknown docking node {}", node.get()),
            Self::NotSplitNode(node) => {
                write!(formatter, "docking node {} is not a split", node.get())
            }
            Self::DuplicateChild(node) => write!(
                formatter,
                "docking node {} cannot be both children of a split",
                node.get()
            ),
            Self::SharedNode(node) => write!(
                formatter,
                "docking node {} is referenced by more than one parent",
                node.get()
            ),
            Self::Cycle(node) => write!(
                formatter,
                "docking node {} creates a cycle in the tree",
                node.get()
            ),
            Self::UnreachableNode(node) => write!(
                formatter,
                "docking node {} is not reachable from the root",
                node.get()
            ),
        }
    }
}

impl Error for DockError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayoutError {
    InvalidBounds(Rect),
    InvalidDividerThickness(f64),
    UnknownNode(NodeId),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds(rect) => write!(formatter, "invalid layout bounds: {rect:?}"),
            Self::InvalidDividerThickness(value) => {
                write!(formatter, "invalid divider thickness: {value}")
            }
            Self::UnknownNode(node) => write!(formatter, "unknown docking node {}", node.get()),
        }
    }
}

impl Error for LayoutError {}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-9;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_rect(actual: Rect, expected: Rect) {
        assert_close(actual.x, expected.x);
        assert_close(actual.y, expected.y);
        assert_close(actual.width, expected.width);
        assert_close(actual.height, expected.height);
    }

    fn tab(builder: &mut DockTreeBuilder, name: &str) -> NodeId {
        builder
            .tabs(vec![PanelId::from(name)], PanelId::from(name))
            .unwrap()
    }

    #[test]
    fn a_single_tab_node_fills_the_bounds() {
        let mut builder = DockTreeBuilder::new();
        let root = tab(&mut builder, "viewport");
        let tree = builder.build(root).unwrap();

        let geometry = compute_geometry(
            &tree,
            Rect::new(10.0, 20.0, 800.0, 600.0),
            LayoutOptions::default(),
        )
        .unwrap();

        assert_eq!(geometry.dividers, vec![]);
        assert_eq!(geometry.panes.len(), 1);
        assert_eq!(geometry.panes[0].node, root);
        assert_rect(geometry.panes[0].rect, Rect::new(10.0, 20.0, 800.0, 600.0));
    }

    #[test]
    fn horizontal_split_runs_left_to_right() {
        let mut builder = DockTreeBuilder::new();
        let left = tab(&mut builder, "viewport");
        let right = tab(&mut builder, "inspector");
        let root = builder.split(Axis::Horizontal, 0.7, left, right).unwrap();
        let tree = builder.build(root).unwrap();

        let geometry = compute_geometry(
            &tree,
            Rect::new(0.0, 0.0, 1_000.0, 500.0),
            LayoutOptions {
                divider_thickness: 10.0,
            },
        )
        .unwrap();

        assert_rect(geometry.panes[0].rect, Rect::new(0.0, 0.0, 693.0, 500.0));
        assert_rect(
            geometry.dividers[0].split_rect,
            Rect::new(0.0, 0.0, 1_000.0, 500.0),
        );
        assert_rect(
            geometry.dividers[0].rect,
            Rect::new(693.0, 0.0, 10.0, 500.0),
        );
        assert_rect(geometry.panes[1].rect, Rect::new(703.0, 0.0, 297.0, 500.0));
    }

    #[test]
    fn vertical_split_runs_top_to_bottom() {
        let mut builder = DockTreeBuilder::new();
        let top = tab(&mut builder, "scene");
        let bottom = tab(&mut builder, "assets");
        let root = builder.split(Axis::Vertical, 0.6, top, bottom).unwrap();
        let tree = builder.build(root).unwrap();

        let geometry = compute_geometry(
            &tree,
            Rect::new(10.0, 20.0, 300.0, 600.0),
            LayoutOptions {
                divider_thickness: 10.0,
            },
        )
        .unwrap();

        assert_rect(geometry.panes[0].rect, Rect::new(10.0, 20.0, 300.0, 354.0));
        assert_rect(
            geometry.dividers[0].rect,
            Rect::new(10.0, 374.0, 300.0, 10.0),
        );
        assert_rect(geometry.panes[1].rect, Rect::new(10.0, 384.0, 300.0, 236.0));
    }

    #[test]
    fn nested_splits_do_not_double_count_dividers() {
        let mut builder = DockTreeBuilder::new();
        let viewport = tab(&mut builder, "viewport");
        let scene = tab(&mut builder, "scene");
        let inspector = tab(&mut builder, "inspector");
        let right = builder
            .split(Axis::Vertical, 0.6, scene, inspector)
            .unwrap();
        let root = builder
            .split(Axis::Horizontal, 0.7, viewport, right)
            .unwrap();
        let tree = builder.build(root).unwrap();

        let geometry = compute_geometry(
            &tree,
            Rect::new(0.0, 0.0, 1_000.0, 600.0),
            LayoutOptions {
                divider_thickness: 10.0,
            },
        )
        .unwrap();

        assert_eq!(geometry.panes.len(), 3);
        assert_eq!(geometry.dividers.len(), 2);
        assert_rect(geometry.panes[0].rect, Rect::new(0.0, 0.0, 693.0, 600.0));
        assert_rect(geometry.panes[1].rect, Rect::new(703.0, 0.0, 297.0, 354.0));
        assert_rect(
            geometry.panes[2].rect,
            Rect::new(703.0, 364.0, 297.0, 236.0),
        );
    }

    #[test]
    fn split_ratios_are_clamped_and_non_finite_values_use_the_default() {
        assert_close(SplitRatio::new(-1.0).get(), MIN_SPLIT_RATIO);
        assert_close(SplitRatio::new(2.0).get(), MAX_SPLIT_RATIO);
        assert_close(SplitRatio::new(f64::NAN).get(), 0.5);
        assert_close(SplitRatio::new(f64::INFINITY).get(), 0.5);
    }

    #[test]
    fn a_split_ratio_can_be_changed_after_building() {
        let mut builder = DockTreeBuilder::new();
        let first = tab(&mut builder, "scene");
        let second = tab(&mut builder, "assets");
        let root = builder.split(Axis::Horizontal, 0.5, first, second).unwrap();
        let mut tree = builder.build(root).unwrap();

        let ratio = tree.set_split_ratio(root, 0.8).unwrap();
        assert_close(ratio.get(), 0.8);

        let geometry = compute_geometry(
            &tree,
            Rect::new(0.0, 0.0, 1_000.0, 500.0),
            LayoutOptions {
                divider_thickness: 10.0,
            },
        )
        .unwrap();
        assert_close(geometry.panes[0].rect.width, 792.0);
    }

    #[test]
    fn tab_nodes_do_not_accept_split_ratios() {
        let mut builder = DockTreeBuilder::new();
        let root = tab(&mut builder, "scene");
        let mut tree = builder.build(root).unwrap();

        assert_eq!(
            tree.set_split_ratio(root, 0.8),
            Err(DockError::NotSplitNode(root))
        );
    }

    #[test]
    fn active_panel_must_be_present_in_the_tab_node() {
        let mut builder = DockTreeBuilder::new();
        let result = builder.tabs(vec![PanelId::from("scene")], PanelId::from("inspector"));

        assert_eq!(
            result,
            Err(DockError::ActivePanelMissing(PanelId::from("inspector")))
        );
    }

    #[test]
    fn every_built_node_must_be_reachable_from_the_root() {
        let mut builder = DockTreeBuilder::new();
        let root = tab(&mut builder, "viewport");
        let orphan = tab(&mut builder, "inspector");

        assert_eq!(builder.build(root), Err(DockError::UnreachableNode(orphan)));
    }

    #[test]
    fn a_node_cannot_be_shared_between_split_branches() {
        let mut builder = DockTreeBuilder::new();
        let shared = tab(&mut builder, "viewport");
        let sibling = tab(&mut builder, "scene");
        let branch = builder
            .split(Axis::Horizontal, 0.5, shared, sibling)
            .unwrap();
        let root = builder.split(Axis::Vertical, 0.5, branch, shared).unwrap();

        assert_eq!(builder.build(root), Err(DockError::SharedNode(shared)));
    }

    #[test]
    fn divider_thickness_is_limited_by_the_available_extent() {
        let mut builder = DockTreeBuilder::new();
        let first = tab(&mut builder, "scene");
        let second = tab(&mut builder, "assets");
        let root = builder.split(Axis::Horizontal, 0.5, first, second).unwrap();
        let tree = builder.build(root).unwrap();

        let geometry = compute_geometry(
            &tree,
            Rect::new(0.0, 0.0, 4.0, 100.0),
            LayoutOptions {
                divider_thickness: 10.0,
            },
        )
        .unwrap();

        assert_rect(geometry.dividers[0].rect, Rect::new(0.0, 0.0, 4.0, 100.0));
        assert_close(geometry.panes[0].rect.width, 0.0);
        assert_close(geometry.panes[1].rect.width, 0.0);
    }
}
