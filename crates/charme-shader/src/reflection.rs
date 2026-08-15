use std::{collections::HashMap, ops::Range};

use naga::{
    AddressSpace, Handle, ImageClass, ImageDimension, ScalarKind, ShaderStage as NagaShaderStage,
    StorageAccess, TypeInner, VectorSize,
    proc::{GlobalCtx, Layouter},
    valid::{Capabilities, GlobalUse, ValidationFlags, Validator},
};
use naga_oil::compose::Composer;

use crate::{
    DeclarationMetadata, MetadataBlock, ModuleMetadata, ScanDiagnosticKind, ShaderComposeError,
    ShaderComposer, ShaderSource, SourceDeclaration,
};

/// Fully composed and validated interface information for one shader variant.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderInterface {
    pub entry_points: Vec<EntryPoint>,
    pub resources: Vec<Resource>,
    pub parameter_blocks: Vec<ParameterBlock>,
    pub diagnostics: Vec<InterfaceDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryPoint {
    pub name: String,
    pub stage: ShaderStage,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Compute,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub name: String,
    pub group: u32,
    pub binding: u32,
    pub kind: ResourceKind,
    pub used_by: Vec<ResourceUse>,
    pub metadata: Option<ReflectedMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceKind {
    UniformBuffer,
    StorageBuffer {
        read: bool,
        write: bool,
    },
    Texture {
        dimension: TextureDimension,
        arrayed: bool,
        multisampled: bool,
        class: TextureClass,
    },
    Sampler {
        comparison: bool,
    },
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureDimension {
    D1,
    D2,
    D3,
    Cube,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureClass {
    Sampled,
    Depth,
    Storage,
    External,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceUse {
    pub entry_point: String,
    pub stage: ShaderStage,
    pub read: bool,
    pub write: bool,
    pub query: bool,
    pub atomic: bool,
}

/// A `var<uniform>` explicitly marked with `reflect.parameters`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterBlock {
    pub name: String,
    pub group: u32,
    pub binding: u32,
    pub size: u32,
    pub alignment: u32,
    pub fields: Vec<ParameterField>,
    pub metadata: ReflectedMetadata,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParameterField {
    pub name: String,
    pub path: Vec<String>,
    pub offset: u32,
    pub size: u32,
    pub alignment: u32,
    pub ty: ParameterType,
    /// False when the source member has `reflect.skip` metadata.
    pub exposed: bool,
    pub metadata: Option<ReflectedMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParameterType {
    Scalar(ScalarType),
    Vector { scalar: ScalarType, length: u8 },
    Unsupported { description: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarType {
    Bool,
    F32,
    I32,
    U32,
}

/// Source documentation carried into the final reflected interface.
#[derive(Clone, Debug, PartialEq)]
pub struct ReflectedMetadata {
    pub module: String,
    pub description: String,
    pub values: MetadataBlock,
    pub source_span: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceDiagnostic {
    pub module: Option<String>,
    pub span: Option<Range<usize>>,
    pub kind: InterfaceDiagnosticKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InterfaceDiagnosticKind {
    SourceMetadata(ScanDiagnosticKind),
    ParameterTargetHasNoBinding {
        name: String,
    },
    ParameterTargetIsNotUniform {
        name: String,
    },
    ParameterTargetIsNotStruct {
        name: String,
    },
    UnsupportedParameterType {
        path: Vec<String>,
        description: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectError {
    pub stage: ReflectStage,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReflectStage {
    Composition,
    Validation,
    Layout,
}

impl std::fmt::Display for ReflectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ReflectError {}

impl From<ShaderComposeError> for ReflectError {
    fn from(error: ShaderComposeError) -> Self {
        Self {
            stage: ReflectStage::Composition,
            message: error.message,
        }
    }
}

impl ShaderComposer {
    /// Composes imports, applies shader defs, validates the resulting Naga
    /// module and reflects its bindable resources and marked parameter blocks.
    pub fn reflect(&mut self, root: &ShaderSource) -> Result<ShaderInterface, ReflectError> {
        let composed = self.compose(root)?;
        reflect_composed(composed)
    }
}

fn reflect_composed(
    composed: crate::composer::ComposedShader,
) -> Result<ShaderInterface, ReflectError> {
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    let module_info = validator
        .validate(&composed.module)
        .map_err(|error| ReflectError {
            stage: ReflectStage::Validation,
            message: error.to_string(),
        })?;

    let mut layouter = Layouter::default();
    layouter
        .update(GlobalCtx {
            types: &composed.module.types,
            constants: &composed.module.constants,
            overrides: &composed.module.overrides,
            global_expressions: &composed.module.global_expressions,
        })
        .map_err(|error| ReflectError {
            stage: ReflectStage::Layout,
            message: error.to_string(),
        })?;

    let annotations = AnnotationIndex::new(&composed.metadata, &composed.root_module);
    let mut diagnostics = source_diagnostics(&composed.metadata);
    let entry_points = composed
        .module
        .entry_points
        .iter()
        .map(|entry| EntryPoint {
            name: entry.name.clone(),
            stage: shader_stage(entry.stage),
        })
        .collect();
    let mut resources = Vec::new();
    let mut parameter_blocks = Vec::new();

    for (global_handle, global) in composed.module.global_variables.iter() {
        let Some(binding) = global.binding.as_ref() else {
            if let Some(annotation) = global
                .name
                .as_deref()
                .and_then(|name| annotations.globals.get(name))
                && annotation.is_parameter_block()
            {
                diagnostics.push(InterfaceDiagnostic {
                    module: Some(annotation.module.to_owned()),
                    span: Some(annotation.declaration.documentation_span.clone()),
                    kind: InterfaceDiagnosticKind::ParameterTargetHasNoBinding {
                        name: annotation.source_name(),
                    },
                });
            }
            continue;
        };
        let ir_name = global
            .name
            .clone()
            .unwrap_or_else(|| format!("binding_{}_{}", binding.group, binding.binding));
        let annotation = annotations.globals.get(&ir_name);
        let name = annotation
            .map(SourceAnnotation::source_name)
            .unwrap_or(ir_name);
        let used_by = resource_uses(&composed.module, &module_info, global_handle);
        resources.push(Resource {
            name: name.clone(),
            group: binding.group,
            binding: binding.binding,
            kind: resource_kind(&composed.module.types[global.ty].inner, global.space),
            used_by,
            metadata: annotation.map(SourceAnnotation::reflected),
        });

        let Some(annotation) = annotation.filter(|annotation| annotation.is_parameter_block())
        else {
            continue;
        };
        if global.space != AddressSpace::Uniform {
            diagnostics.push(InterfaceDiagnostic {
                module: Some(annotation.module.to_owned()),
                span: Some(annotation.declaration.documentation_span.clone()),
                kind: InterfaceDiagnosticKind::ParameterTargetIsNotUniform { name },
            });
            continue;
        }
        let TypeInner::Struct { members, .. } = &composed.module.types[global.ty].inner else {
            diagnostics.push(InterfaceDiagnostic {
                module: Some(annotation.module.to_owned()),
                span: Some(annotation.declaration.documentation_span.clone()),
                kind: InterfaceDiagnosticKind::ParameterTargetIsNotStruct { name },
            });
            continue;
        };

        let type_name = composed.module.types[global.ty].name.as_deref();
        let fields = members
            .iter()
            .enumerate()
            .map(|(member_index, member)| {
                let field_name = member
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("member_{member_index}"));
                let field_annotation = type_name.and_then(|type_name| {
                    annotations
                        .members
                        .get(&(type_name.to_owned(), field_name.clone()))
                });
                let layout = layouter[member.ty];
                let ty = parameter_type(&composed.module.types[member.ty].inner);
                if let ParameterType::Unsupported { description } = &ty {
                    diagnostics.push(InterfaceDiagnostic {
                        module: field_annotation.map(|value| value.module.to_owned()),
                        span: field_annotation
                            .map(|value| value.declaration.documentation_span.clone()),
                        kind: InterfaceDiagnosticKind::UnsupportedParameterType {
                            path: vec![field_name.clone()],
                            description: description.clone(),
                        },
                    });
                }
                ParameterField {
                    name: field_name.clone(),
                    path: vec![field_name],
                    offset: member.offset,
                    size: layout.size,
                    alignment: layout.alignment * 1,
                    ty,
                    exposed: !field_annotation.is_some_and(|value| value.is_skipped()),
                    metadata: field_annotation.map(SourceAnnotation::reflected),
                }
            })
            .collect();
        let block_layout = layouter[global.ty];
        parameter_blocks.push(ParameterBlock {
            name,
            group: binding.group,
            binding: binding.binding,
            size: block_layout.size,
            alignment: block_layout.alignment * 1,
            fields,
            metadata: annotation.reflected(),
        });
    }

    Ok(ShaderInterface {
        entry_points,
        resources,
        parameter_blocks,
        diagnostics,
    })
}

fn resource_uses(
    module: &naga::Module,
    module_info: &naga::valid::ModuleInfo,
    global: Handle<naga::GlobalVariable>,
) -> Vec<ResourceUse> {
    module
        .entry_points
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let usage = module_info.get_entry_point(index)[global];
            (!usage.is_empty()).then(|| ResourceUse {
                entry_point: entry.name.clone(),
                stage: shader_stage(entry.stage),
                read: usage.contains(GlobalUse::READ),
                write: usage.contains(GlobalUse::WRITE),
                query: usage.contains(GlobalUse::QUERY),
                atomic: usage.contains(GlobalUse::ATOMIC),
            })
        })
        .collect()
}

fn resource_kind(inner: &TypeInner, space: AddressSpace) -> ResourceKind {
    match space {
        AddressSpace::Uniform => ResourceKind::UniformBuffer,
        AddressSpace::Storage { access } => ResourceKind::StorageBuffer {
            read: access.contains(StorageAccess::LOAD),
            write: access.contains(StorageAccess::STORE),
        },
        _ => match inner {
            TypeInner::Image {
                dim,
                arrayed,
                class,
            } => ResourceKind::Texture {
                dimension: texture_dimension(*dim),
                arrayed: *arrayed,
                multisampled: match class {
                    ImageClass::Sampled { multi, .. } | ImageClass::Depth { multi } => *multi,
                    ImageClass::Storage { .. } | ImageClass::External => false,
                },
                class: match class {
                    ImageClass::Sampled { .. } => TextureClass::Sampled,
                    ImageClass::Depth { .. } => TextureClass::Depth,
                    ImageClass::Storage { .. } => TextureClass::Storage,
                    ImageClass::External => TextureClass::External,
                },
            },
            TypeInner::Sampler { comparison } => ResourceKind::Sampler {
                comparison: *comparison,
            },
            _ => ResourceKind::Other,
        },
    }
}

fn parameter_type(inner: &TypeInner) -> ParameterType {
    match inner {
        TypeInner::Scalar(scalar) => scalar_type(scalar.kind, scalar.width)
            .map(ParameterType::Scalar)
            .unwrap_or_else(|| ParameterType::Unsupported {
                description: type_description(inner),
            }),
        TypeInner::Vector { size, scalar } => scalar_type(scalar.kind, scalar.width)
            .map(|scalar| ParameterType::Vector {
                scalar,
                length: vector_length(*size),
            })
            .unwrap_or_else(|| ParameterType::Unsupported {
                description: type_description(inner),
            }),
        _ => ParameterType::Unsupported {
            description: type_description(inner),
        },
    }
}

fn scalar_type(kind: ScalarKind, width: u8) -> Option<ScalarType> {
    match (kind, width) {
        (ScalarKind::Bool, 1 | 4) => Some(ScalarType::Bool),
        (ScalarKind::Float, 4) => Some(ScalarType::F32),
        (ScalarKind::Sint, 4) => Some(ScalarType::I32),
        (ScalarKind::Uint, 4) => Some(ScalarType::U32),
        _ => None,
    }
}

fn type_description(inner: &TypeInner) -> String {
    match inner {
        TypeInner::Scalar(scalar) => format!("{:?} scalar ({} bytes)", scalar.kind, scalar.width),
        TypeInner::Vector { size, scalar } => format!(
            "vec{}<{:?}, {} bytes>",
            vector_length(*size),
            scalar.kind,
            scalar.width
        ),
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => format!(
            "mat{}x{}<{:?}>",
            vector_length(*columns),
            vector_length(*rows),
            scalar.kind
        ),
        TypeInner::Array { .. } => "array".to_owned(),
        TypeInner::Struct { .. } => "nested struct".to_owned(),
        other => format!("{other:?}"),
    }
}

fn vector_length(size: VectorSize) -> u8 {
    match size {
        VectorSize::Bi => 2,
        VectorSize::Tri => 3,
        VectorSize::Quad => 4,
    }
}

fn texture_dimension(dimension: ImageDimension) -> TextureDimension {
    match dimension {
        ImageDimension::D1 => TextureDimension::D1,
        ImageDimension::D2 => TextureDimension::D2,
        ImageDimension::D3 => TextureDimension::D3,
        ImageDimension::Cube => TextureDimension::Cube,
    }
}

fn shader_stage(stage: NagaShaderStage) -> ShaderStage {
    match stage {
        NagaShaderStage::Vertex => ShaderStage::Vertex,
        NagaShaderStage::Fragment => ShaderStage::Fragment,
        NagaShaderStage::Compute => ShaderStage::Compute,
        _ => ShaderStage::Other,
    }
}

fn source_diagnostics(modules: &[ModuleMetadata]) -> Vec<InterfaceDiagnostic> {
    modules
        .iter()
        .flat_map(|module| {
            module
                .diagnostics
                .iter()
                .map(|diagnostic| InterfaceDiagnostic {
                    module: Some(module.module.clone()),
                    span: Some(diagnostic.span.clone()),
                    kind: InterfaceDiagnosticKind::SourceMetadata(diagnostic.kind.clone()),
                })
        })
        .collect()
}

struct AnnotationIndex<'a> {
    globals: HashMap<String, SourceAnnotation<'a>>,
    members: HashMap<(String, String), SourceAnnotation<'a>>,
}

impl<'a> AnnotationIndex<'a> {
    fn new(modules: &'a [ModuleMetadata], root_module: &str) -> Self {
        let mut globals = HashMap::new();
        let mut members = HashMap::new();

        for (module_index, module) in modules.iter().enumerate() {
            let is_root = module_index == 0 && module.module == root_module;
            for declaration in &module.declarations {
                match &declaration.declaration {
                    SourceDeclaration::GlobalVariable { name } => {
                        globals.insert(
                            final_name(is_root, &module.module, name),
                            SourceAnnotation {
                                module: &module.module,
                                declaration,
                            },
                        );
                    }
                    SourceDeclaration::StructMember { structure, member } => {
                        members.insert(
                            (
                                final_name(is_root, &module.module, structure),
                                member.clone(),
                            ),
                            SourceAnnotation {
                                module: &module.module,
                                declaration,
                            },
                        );
                    }
                    SourceDeclaration::Struct { .. }
                    | SourceDeclaration::Override { .. }
                    | SourceDeclaration::Constant { .. } => {}
                }
            }
        }

        Self { globals, members }
    }
}

struct SourceAnnotation<'a> {
    module: &'a str,
    declaration: &'a DeclarationMetadata,
}

impl SourceAnnotation<'_> {
    fn source_name(&self) -> String {
        match &self.declaration.declaration {
            SourceDeclaration::GlobalVariable { name }
            | SourceDeclaration::Struct { name }
            | SourceDeclaration::Override { name }
            | SourceDeclaration::Constant { name } => name.clone(),
            SourceDeclaration::StructMember { member, .. } => member.clone(),
        }
    }

    fn reflected(&self) -> ReflectedMetadata {
        ReflectedMetadata {
            module: self.module.to_owned(),
            description: self.declaration.description.clone(),
            values: self.declaration.metadata.clone(),
            source_span: self.declaration.documentation_span.clone(),
        }
    }

    fn is_parameter_block(&self) -> bool {
        metadata_flag(&self.declaration.metadata, "reflect.parameters")
    }

    fn is_skipped(&self) -> bool {
        metadata_flag(&self.declaration.metadata, "reflect.skip")
    }
}

fn metadata_flag(metadata: &MetadataBlock, path: &str) -> bool {
    matches!(metadata.get(path), Some(crate::MetadataValue::Bool(true)))
}

fn final_name(root: bool, module: &str, name: &str) -> String {
    if root {
        name.to_owned()
    } else {
        Composer::decorated_name(Some(module), name)
    }
}
