use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A file reference stored either relative to the Charme project or as an
/// explicit absolute path.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ResourcePath {
    /// A portable path resolved relative to the project file's parent directory.
    ProjectRelative(PathBuf),
    /// A machine-local absolute path.
    Absolute(PathBuf),
}

impl ResourcePath {
    /// Creates a normalized project-relative path.
    ///
    /// Parent components that escape the project directory are rejected.
    pub fn project_relative(path: impl AsRef<Path>) -> Result<Self, ResourcePathError> {
        Ok(Self::ProjectRelative(normalize_relative(path.as_ref())?))
    }

    /// Creates an absolute machine-local path.
    pub fn absolute(path: impl Into<PathBuf>) -> Result<Self, ResourcePathError> {
        let path = path.into();
        if !path.is_absolute() {
            return Err(ResourcePathError::ExpectedAbsolute(path));
        }
        Ok(Self::Absolute(path))
    }

    /// Chooses a project-relative reference when `path` is inside
    /// `project_directory`, and an absolute reference otherwise.
    pub fn from_path(
        project_directory: &Path,
        path: impl AsRef<Path>,
    ) -> Result<Self, ResourcePathError> {
        let path = path.as_ref();
        if path.is_relative() {
            return Self::project_relative(path);
        }
        if let Ok(relative) = path.strip_prefix(project_directory) {
            return Self::project_relative(relative);
        }
        Self::absolute(path.to_path_buf())
    }

    /// Resolves this reference to a file-system path.
    pub fn resolve(&self, project_directory: &Path) -> PathBuf {
        match self {
            Self::ProjectRelative(path) => project_directory.join(path),
            Self::Absolute(path) => path.clone(),
        }
    }

    /// Returns the stored path without resolving it.
    pub fn stored_path(&self) -> &Path {
        match self {
            Self::ProjectRelative(path) | Self::Absolute(path) => path,
        }
    }

    /// Returns true when the path can move with the project directory.
    pub const fn is_project_relative(&self) -> bool {
        matches!(self, Self::ProjectRelative(_))
    }
}

/// An invalid resource path.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResourcePathError {
    /// A project-relative path was empty.
    #[error("a project-relative resource path cannot be empty")]
    Empty,
    /// A project-relative path was absolute.
    #[error("expected a project-relative path, got {}", .0.display())]
    ExpectedRelative(PathBuf),
    /// An absolute path was relative.
    #[error("expected an absolute path, got {}", .0.display())]
    ExpectedAbsolute(PathBuf),
    /// A parent component escaped the project directory.
    #[error("project-relative path escapes the project directory: {}", .0.display())]
    EscapesProject(PathBuf),
}

fn normalize_relative(path: &Path) -> Result<PathBuf, ResourcePathError> {
    if path.as_os_str().is_empty() {
        return Err(ResourcePathError::Empty);
    }
    if path.is_absolute() {
        return Err(ResourcePathError::ExpectedRelative(path.to_path_buf()));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(ResourcePathError::EscapesProject(path.to_path_buf()));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ResourcePathError::ExpectedRelative(path.to_path_buf()));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(ResourcePathError::Empty);
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_safe_relative_paths() {
        let path = ResourcePath::project_relative("models/parts/../character.pmx").unwrap();
        assert_eq!(path.stored_path(), Path::new("models/character.pmx"));
        assert_eq!(
            path.resolve(Path::new("/project")),
            PathBuf::from("/project/models/character.pmx")
        );
    }

    #[test]
    fn rejects_paths_that_escape_the_project() {
        assert!(matches!(
            ResourcePath::project_relative("../character.pmx"),
            Err(ResourcePathError::EscapesProject(_))
        ));
    }

    #[test]
    fn chooses_relative_paths_inside_the_project() {
        let reference =
            ResourcePath::from_path(Path::new("/project"), "/project/shaders/toon.wgsl").unwrap();
        assert_eq!(
            reference,
            ResourcePath::ProjectRelative(PathBuf::from("shaders/toon.wgsl"))
        );
    }
}
