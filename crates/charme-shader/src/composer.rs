use std::collections::{BTreeMap, HashMap};

use naga_oil::compose::{
    ComposableModuleDescriptor, Composer, NagaModuleDescriptor, ShaderLanguage, ShaderType,
    preprocess::Preprocessor,
};

use crate::{ModuleMetadata, scan_module_metadata};

/// A value used by naga-oil conditional preprocessing.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ShaderDefValue {
    Bool(bool),
    Int(i32),
    UInt(u32),
}

pub type ShaderDefs = BTreeMap<String, ShaderDefValue>;

/// WGSL source and the variant defaults used to compose it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShaderSource {
    pub source: String,
    pub file_path: String,
    /// Optional import name override for a composable module.
    pub module_name: Option<String>,
    pub shader_defs: ShaderDefs,
}

impl ShaderSource {
    pub fn new(source: impl Into<String>, file_path: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            file_path: file_path.into(),
            module_name: None,
            shader_defs: ShaderDefs::new(),
        }
    }

    pub fn with_module_name(mut self, module_name: impl Into<String>) -> Self {
        self.module_name = Some(module_name.into());
        self
    }

    pub fn with_shader_def(mut self, name: impl Into<String>, value: ShaderDefValue) -> Self {
        self.shader_defs.insert(name.into(), value);
        self
    }
}

/// Stateful naga-oil composer plus source metadata needed for reflection.
pub struct ShaderComposer {
    pub(crate) composer: Composer,
    modules: Vec<RegisteredModule>,
}

impl std::fmt::Debug for ShaderComposer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShaderComposer")
            .field("registered_modules", &self.modules.len())
            .finish_non_exhaustive()
    }
}

impl Default for ShaderComposer {
    fn default() -> Self {
        Self::new()
    }
}

impl ShaderComposer {
    pub fn new() -> Self {
        Self {
            composer: Composer::default(),
            modules: Vec::new(),
        }
    }

    /// Adds a composable WGSL module. Dependencies must be added first, as
    /// required by naga-oil.
    pub fn add_composable_module(
        &mut self,
        module: ShaderSource,
    ) -> Result<String, ShaderComposeError> {
        let shader_defs = to_naga_defs(&module.shader_defs);
        let name = self
            .composer
            .add_composable_module(ComposableModuleDescriptor {
                source: &module.source,
                file_path: &module.file_path,
                language: ShaderLanguage::Wgsl,
                as_name: module.module_name.clone(),
                additional_imports: &[],
                shader_defs,
            })
            .map(|definition| definition.name.clone())
            .map_err(|error| ShaderComposeError::composer(&self.composer, error))?;

        self.modules.retain(|registered| registered.name != name);
        self.modules.push(RegisteredModule {
            name: name.clone(),
            source: module,
        });
        Ok(name)
    }

    pub(crate) fn compose(
        &mut self,
        root: &ShaderSource,
    ) -> Result<ComposedShader, ShaderComposeError> {
        let shader_defs = to_naga_defs(&root.shader_defs);
        let module = self
            .composer
            .make_naga_module(NagaModuleDescriptor {
                source: &root.source,
                file_path: &root.file_path,
                shader_type: ShaderType::Wgsl,
                shader_defs: shader_defs.clone(),
                additional_imports: &[],
            })
            .map_err(|error| ShaderComposeError::composer(&self.composer, error))?;

        let mut metadata = Vec::with_capacity(self.modules.len() + 1);
        metadata.push(scan_variant(
            &root.file_path,
            &root.source,
            &root.shader_defs,
        )?);
        for registered in &self.modules {
            let mut effective_defs = root.shader_defs.clone();
            effective_defs.extend(registered.source.shader_defs.clone());
            metadata.push(scan_variant(
                &registered.name,
                &registered.source.source,
                &effective_defs,
            )?);
        }

        Ok(ComposedShader {
            module,
            root_module: root.file_path.clone(),
            metadata,
        })
    }
}

pub(crate) struct ComposedShader {
    pub module: naga::Module,
    pub root_module: String,
    pub metadata: Vec<ModuleMetadata>,
}

struct RegisteredModule {
    name: String,
    source: ShaderSource,
}

fn scan_variant(
    module_name: &str,
    source: &str,
    shader_defs: &ShaderDefs,
) -> Result<ModuleMetadata, ShaderComposeError> {
    let preprocessor = Preprocessor::default();
    let metadata = preprocessor
        .get_preprocessor_metadata(source, true)
        .map_err(|error| ShaderComposeError::preprocess(module_name, error.to_string()))?;
    let mut effective_defs = to_naga_defs(shader_defs);
    effective_defs.extend(metadata.defines);
    let output = preprocessor
        .preprocess(&metadata.cleaned_source, &effective_defs)
        .map_err(|error| ShaderComposeError::preprocess(module_name, error.to_string()))?;
    Ok(scan_module_metadata(
        module_name,
        &output.preprocessed_source,
    ))
}

fn to_naga_defs(defs: &ShaderDefs) -> HashMap<String, naga_oil::compose::ShaderDefValue> {
    defs.iter()
        .map(|(name, value)| {
            let value = match value {
                ShaderDefValue::Bool(value) => naga_oil::compose::ShaderDefValue::Bool(*value),
                ShaderDefValue::Int(value) => naga_oil::compose::ShaderDefValue::Int(*value),
                ShaderDefValue::UInt(value) => naga_oil::compose::ShaderDefValue::UInt(*value),
            };
            (name.clone(), value)
        })
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShaderComposeError {
    pub stage: ComposeStage,
    pub message: String,
}

impl ShaderComposeError {
    fn composer(composer: &Composer, error: naga_oil::compose::ComposerError) -> Self {
        Self {
            stage: ComposeStage::Composition,
            message: error.emit_to_string(composer),
        }
    }

    fn preprocess(module: &str, message: String) -> Self {
        Self {
            stage: ComposeStage::MetadataPreprocessing,
            message: format!("failed to preprocess metadata source `{module}`: {message}"),
        }
    }
}

impl std::fmt::Display for ShaderComposeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ShaderComposeError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposeStage {
    Composition,
    MetadataPreprocessing,
}
