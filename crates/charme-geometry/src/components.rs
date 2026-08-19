use std::collections::{HashMap, HashSet};

use thiserror::Error;

/// Defines which indexed-triangle relationships make two triangles connected.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Connectivity {
    /// Connects triangles that share at least one indexed vertex.
    SharedVertex,
    /// Connects triangles that share the same two indexed vertices, regardless of winding.
    #[default]
    SharedEdge,
}

/// Describes the contiguous portion of an index buffer occupied by one primitive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrimitiveRange {
    /// Offset of the first index in the source index buffer.
    pub index_start: usize,
    /// Number of indices occupied by the primitive.
    pub index_count: usize,
}

impl PrimitiveRange {
    /// Creates a primitive index range.
    pub const fn new(index_start: usize, index_count: usize) -> Self {
        Self {
            index_start,
            index_count,
        }
    }
}

/// One connected component of a primitive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeshComponent {
    /// Zero-based triangle ordinals relative to the input primitive, in source order.
    pub triangle_indices: Vec<usize>,
    /// The original vertex indices for the component's triangles, in source order.
    pub indices: Vec<u32>,
    /// Distinct original vertex indices referenced by [`Self::indices`], in first-use order.
    pub vertex_indices: Vec<u32>,
}

impl MeshComponent {
    /// Returns the number of triangles in the component.
    pub fn triangle_count(&self) -> usize {
        self.triangle_indices.len()
    }

    /// Returns the number of indices in the component's triangle list.
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }
}

/// The connected-component partition of one primitive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrimitiveSplit {
    /// The source primitive range used for this partition.
    pub range: PrimitiveRange,
    /// The connectivity rule used to produce the partition.
    pub connectivity: Connectivity,
    /// Component ID for each source triangle, indexed by local triangle ordinal.
    pub triangle_components: Vec<usize>,
    /// Components in the order of the first source triangle they contain.
    pub components: Vec<MeshComponent>,
}

impl PrimitiveSplit {
    /// Returns the component ID for a local triangle ordinal.
    pub fn component_for_triangle(&self, triangle_index: usize) -> Option<usize> {
        self.triangle_components.get(triangle_index).copied()
    }

    /// Returns the number of source triangles in the primitive.
    pub fn triangle_count(&self) -> usize {
        self.triangle_components.len()
    }
}

/// An error encountered while validating a primitive index range.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SplitError {
    /// The primitive index count does not contain complete triangles.
    #[error("primitive index count {index_count} is not divisible by 3")]
    IndexCountNotTriangular {
        /// Number of indices in the invalid primitive range.
        index_count: usize,
    },
    /// Adding the primitive start and count overflowed `usize`.
    #[error(
        "primitive index range starting at {index_start} with count {index_count} overflows usize"
    )]
    IndexRangeOverflow {
        /// Primitive start offset.
        index_start: usize,
        /// Primitive index count.
        index_count: usize,
    },
    /// The primitive range lies outside the source index buffer.
    #[error(
        "primitive index range {index_start}..{index_end} exceeds index buffer length {index_buffer_length}"
    )]
    IndexRangeOutOfBounds {
        /// Primitive start offset.
        index_start: usize,
        /// Exclusive primitive end offset.
        index_end: usize,
        /// Source index buffer length.
        index_buffer_length: usize,
    },
    /// A primitive references a vertex outside the supplied vertex buffer.
    #[error(
        "vertex index {vertex_index} at index offset {index_offset} exceeds vertex buffer length {vertex_count}"
    )]
    InvalidVertexIndex {
        /// The invalid source vertex index.
        vertex_index: u32,
        /// Offset of the invalid index in the source index buffer.
        index_offset: usize,
        /// Number of vertices in the source vertex buffer.
        vertex_count: usize,
    },
}

/// Splits a primitive using shared-edge connectivity.
///
/// This is the recommended default for surface-part separation: triangles that
/// only touch at one vertex remain in separate components.
///
/// The returned component indices refer to the original vertex buffer, so the
/// caller can reuse positions, normals, UVs, skinning data, and other
/// attributes without the algorithm copying or rewriting them.
pub fn split_primitive(
    indices: &[u32],
    vertex_count: usize,
    range: PrimitiveRange,
) -> Result<PrimitiveSplit, SplitError> {
    split_primitive_with_connectivity(indices, vertex_count, range, Connectivity::default())
}

/// Splits a primitive using an explicit connectivity rule.
///
/// The primitive range must contain a whole number of triangles. Every index is
/// checked against `vertex_count` before the topology is analyzed.
pub fn split_primitive_with_connectivity(
    indices: &[u32],
    vertex_count: usize,
    range: PrimitiveRange,
    connectivity: Connectivity,
) -> Result<PrimitiveSplit, SplitError> {
    if !range.index_count.is_multiple_of(3) {
        return Err(SplitError::IndexCountNotTriangular {
            index_count: range.index_count,
        });
    }

    let index_end =
        range
            .index_start
            .checked_add(range.index_count)
            .ok_or(SplitError::IndexRangeOverflow {
                index_start: range.index_start,
                index_count: range.index_count,
            })?;
    if index_end > indices.len() {
        return Err(SplitError::IndexRangeOutOfBounds {
            index_start: range.index_start,
            index_end,
            index_buffer_length: indices.len(),
        });
    }

    let primitive_indices = &indices[range.index_start..index_end];
    for (offset, &vertex_index) in primitive_indices.iter().enumerate() {
        let valid = usize::try_from(vertex_index)
            .ok()
            .is_some_and(|index| index < vertex_count);
        if !valid {
            return Err(SplitError::InvalidVertexIndex {
                vertex_index,
                index_offset: range.index_start + offset,
                vertex_count,
            });
        }
    }

    let triangles = primitive_indices
        .chunks_exact(3)
        .map(|triangle| [triangle[0], triangle[1], triangle[2]])
        .collect::<Vec<_>>();
    let mut sets = DisjointSets::new(triangles.len());

    match connectivity {
        Connectivity::SharedVertex => connect_by_vertex(&triangles, &mut sets),
        Connectivity::SharedEdge => connect_by_edge(&triangles, &mut sets),
    }

    let mut components = Vec::new();
    let mut component_by_root = HashMap::<usize, usize>::new();
    let mut triangle_components = Vec::with_capacity(triangles.len());

    for triangle_index in 0..triangles.len() {
        let root = sets.find(triangle_index);
        let component_index = if let Some(&component_index) = component_by_root.get(&root) {
            component_index
        } else {
            let component_index = components.len();
            component_by_root.insert(root, component_index);
            components.push(MeshComponent {
                triangle_indices: Vec::new(),
                indices: Vec::new(),
                vertex_indices: Vec::new(),
            });
            component_index
        };
        components[component_index]
            .triangle_indices
            .push(triangle_index);
        triangle_components.push(component_index);
    }

    for component in &mut components {
        let mut seen_vertices = HashSet::new();
        for &triangle_index in &component.triangle_indices {
            let triangle_start = triangle_index * 3;
            let triangle = &primitive_indices[triangle_start..triangle_start + 3];
            component.indices.extend_from_slice(triangle);
            for &vertex_index in triangle {
                if seen_vertices.insert(vertex_index) {
                    component.vertex_indices.push(vertex_index);
                }
            }
        }
    }

    Ok(PrimitiveSplit {
        range,
        connectivity,
        triangle_components,
        components,
    })
}

fn connect_by_vertex(triangles: &[[u32; 3]], sets: &mut DisjointSets) {
    let mut first_triangle_by_vertex = HashMap::<u32, usize>::new();
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        for &vertex in triangle {
            if let Some(&first_triangle) = first_triangle_by_vertex.get(&vertex) {
                sets.union(first_triangle, triangle_index);
            } else {
                first_triangle_by_vertex.insert(vertex, triangle_index);
            }
        }
    }
}

fn connect_by_edge(triangles: &[[u32; 3]], sets: &mut DisjointSets) {
    let mut first_triangle_by_edge = HashMap::<Edge, usize>::new();
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        for edge in [
            Edge::new(triangle[0], triangle[1]),
            Edge::new(triangle[1], triangle[2]),
            Edge::new(triangle[2], triangle[0]),
        ] {
            if let Some(&first_triangle) = first_triangle_by_edge.get(&edge) {
                sets.union(first_triangle, triangle_index);
            } else {
                first_triangle_by_edge.insert(edge, triangle_index);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Edge {
    first: u32,
    second: u32,
}

impl Edge {
    fn new(first: u32, second: u32) -> Self {
        if first <= second {
            Self { first, second }
        } else {
            Self {
                first: second,
                second: first,
            }
        }
    }
}

struct DisjointSets {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl DisjointSets {
    fn new(length: usize) -> Self {
        Self {
            parent: (0..length).collect(),
            rank: vec![0; length],
        }
    }

    fn find(&mut self, element: usize) -> usize {
        let parent = self.parent[element];
        if parent == element {
            element
        } else {
            let root = self.find(parent);
            self.parent[element] = root;
            root
        }
    }

    fn union(&mut self, first: usize, second: usize) {
        let first_root = self.find(first);
        let second_root = self.find(second);
        if first_root == second_root {
            return;
        }

        match self.rank[first_root].cmp(&self.rank[second_root]) {
            std::cmp::Ordering::Less => self.parent[first_root] = second_root,
            std::cmp::Ordering::Greater => self.parent[second_root] = first_root,
            std::cmp::Ordering::Equal => {
                self.parent[second_root] = first_root;
                self.rank[first_root] += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Connectivity, MeshComponent, PrimitiveRange, SplitError, split_primitive,
        split_primitive_with_connectivity,
    };

    #[test]
    fn shared_edges_form_components_in_source_order() {
        let indices = [0, 1, 2, 2, 1, 3, 4, 5, 6];

        let split = split_primitive(&indices, 7, PrimitiveRange::new(0, indices.len()))
            .expect("valid primitive should split");

        assert_eq!(split.connectivity, Connectivity::SharedEdge);
        assert_eq!(split.triangle_components, [0, 0, 1]);
        assert_eq!(
            split.components,
            [
                MeshComponent {
                    triangle_indices: vec![0, 1],
                    indices: vec![0, 1, 2, 2, 1, 3],
                    vertex_indices: vec![0, 1, 2, 3],
                },
                MeshComponent {
                    triangle_indices: vec![2],
                    indices: vec![4, 5, 6],
                    vertex_indices: vec![4, 5, 6],
                },
            ]
        );
    }

    #[test]
    fn shared_vertex_connectivity_can_join_point_touching_triangles() {
        let indices = [0, 1, 2, 2, 3, 4];

        let split = split_primitive_with_connectivity(
            &indices,
            5,
            PrimitiveRange::new(0, indices.len()),
            Connectivity::SharedVertex,
        )
        .expect("valid primitive should split");

        assert_eq!(split.triangle_components, [0, 0]);
        assert_eq!(split.components[0].triangle_count(), 2);
    }

    #[test]
    fn primitive_range_isolated_from_other_triangles() {
        let indices = [0, 1, 2, 0, 2, 3, 0, 4, 5];

        let split = split_primitive(&indices, 6, PrimitiveRange::new(3, 6))
            .expect("valid primitive should split");

        assert_eq!(split.triangle_components, [0, 1]);
        assert_eq!(split.components[0].indices, [0, 2, 3]);
        assert_eq!(split.components[1].indices, [0, 4, 5]);
    }

    #[test]
    fn empty_primitive_has_no_components() {
        let split = split_primitive(&[], 0, PrimitiveRange::new(0, 0))
            .expect("empty primitive should be valid");

        assert!(split.components.is_empty());
        assert!(split.triangle_components.is_empty());
        assert_eq!(split.triangle_count(), 0);
        assert_eq!(split.component_for_triangle(0), None);
    }

    #[test]
    fn rejects_non_triangular_ranges() {
        let error = split_primitive(&[0, 1, 2, 3], 4, PrimitiveRange::new(0, 4))
            .expect_err("partial triangle should be rejected");

        assert_eq!(
            error,
            SplitError::IndexCountNotTriangular { index_count: 4 }
        );
    }

    #[test]
    fn rejects_ranges_outside_index_buffer() {
        let error = split_primitive(&[0, 1, 2], 3, PrimitiveRange::new(1, 3))
            .expect_err("out-of-bounds range should be rejected");

        assert_eq!(
            error,
            SplitError::IndexRangeOutOfBounds {
                index_start: 1,
                index_end: 4,
                index_buffer_length: 3,
            }
        );
    }

    #[test]
    fn rejects_invalid_vertex_indices() {
        let error = split_primitive(&[0, 1, 3], 3, PrimitiveRange::new(0, 3))
            .expect_err("invalid vertex index should be rejected");

        assert_eq!(
            error,
            SplitError::InvalidVertexIndex {
                vertex_index: 3,
                index_offset: 2,
                vertex_count: 3,
            }
        );
    }

    #[test]
    fn rejects_range_overflow() {
        let error = split_primitive(&[], 0, PrimitiveRange::new(usize::MAX, 3))
            .expect_err("overflowing range should be rejected");

        assert_eq!(
            error,
            SplitError::IndexRangeOverflow {
                index_start: usize::MAX,
                index_count: 3,
            }
        );
    }
}
