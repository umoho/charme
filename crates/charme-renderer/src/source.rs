use std::{
    io,
    path::{Path, PathBuf},
};

use bevy_pmx::{PmxResolvedPath, PmxSource, PmxSourceLocation};
use charme_core::MaterialSlotId;

use crate::archive::{
    archive_root, discover_pmx_archive_entries, is_zip_path, normalize_archive_entry,
};

/// Identifies one PMX input independently of how it is read at runtime.
///
/// For a regular PMX file, `path` is the PMX path and `archive_entry` is
/// `None`. For a PMX stored in a ZIP package, `path` is the archive path and
/// `archive_entry` is the normalized entry containing the PMX document.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PmxSourceIdentity {
    path: PathBuf,
    archive_entry: Option<String>,
}

impl PmxSourceIdentity {
    /// Creates an identity from a file or archive path and an optional PMX entry.
    pub fn new(path: impl Into<PathBuf>, archive_entry: Option<String>) -> Self {
        let archive_entry =
            archive_entry.map(|entry| normalize_archive_entry(&entry).unwrap_or(entry));
        Self {
            path: path.into(),
            archive_entry,
        }
    }

    /// Creates the identity of a PMX file on disk.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::new(path, None)
    }

    /// Creates the identity of a PMX entry inside a ZIP archive.
    pub fn zip(path: impl Into<PathBuf>, archive_entry: impl Into<String>) -> Self {
        Self::new(path, Some(archive_entry.into()))
    }

    /// Returns the PMX path or the containing ZIP archive path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the selected PMX entry inside the ZIP archive, if any.
    pub fn archive_entry(&self) -> Option<&str> {
        self.archive_entry.as_deref()
    }
}

/// A source-specific runtime reader used by the PMX import pipeline.
///
/// Implementations retain the corresponding `bevy_pmx` source and location so
/// the common importer can reuse its relative-texture resolver and byte
/// reader without knowing whether the input came from a folder or a ZIP.
pub(crate) trait PmxInputSource: Send {
    fn identity(&self) -> &PmxSourceIdentity;
    fn bevy_source(&self) -> &PmxSource;
    fn pmx_location(&self) -> &PmxSourceLocation;

    fn read_pmx_bytes(&self) -> io::Result<Vec<u8>> {
        self.bevy_source().read_bytes(self.pmx_location())
    }

    fn read_texture_bytes(&self, path: &PmxResolvedPath) -> io::Result<Vec<u8>> {
        self.bevy_source().read_bytes(path.location())
    }
}

struct FilePmxSource {
    identity: PmxSourceIdentity,
    source: PmxSource,
    location: PmxSourceLocation,
}

impl FilePmxSource {
    fn new(path: PathBuf) -> Self {
        let source = PmxSource::folder(path.parent().unwrap_or_else(|| Path::new(".")));
        let location = PmxSourceLocation::disk(path.clone());
        Self {
            identity: PmxSourceIdentity::file(path),
            source,
            location,
        }
    }
}

impl PmxInputSource for FilePmxSource {
    fn identity(&self) -> &PmxSourceIdentity {
        &self.identity
    }

    fn bevy_source(&self) -> &PmxSource {
        &self.source
    }

    fn pmx_location(&self) -> &PmxSourceLocation {
        &self.location
    }
}

struct ZippedPmxSource {
    identity: PmxSourceIdentity,
    source: PmxSource,
    location: PmxSourceLocation,
}

impl ZippedPmxSource {
    fn new(archive: PathBuf, entry: String) -> Self {
        let source = PmxSource::zip_with_encoding(
            archive.clone(),
            archive_root(&entry),
            bevy_pmx::ZipNameEncoding::Auto,
        );
        let location = PmxSourceLocation::zip(archive.clone(), entry.clone());
        Self {
            identity: PmxSourceIdentity::zip(archive, entry),
            source,
            location,
        }
    }
}

impl PmxInputSource for ZippedPmxSource {
    fn identity(&self) -> &PmxSourceIdentity {
        &self.identity
    }

    fn bevy_source(&self) -> &PmxSource {
        &self.source
    }

    fn pmx_location(&self) -> &PmxSourceLocation {
        &self.location
    }
}

#[derive(Debug)]
enum PmxSourceRequest {
    Path {
        path: PathBuf,
        archive_entry: Option<String>,
    },
}

/// A PMX loading request containing its source and existing material-slot IDs.
///
/// The request is a runtime value and is intentionally not serializable. Use
/// [`charme_core::CharacterSource`] for the persistent project description.
#[derive(Debug)]
pub struct PmxLoadRequest {
    source: PmxSourceRequest,
    existing_slot_ids: Vec<(u32, MaterialSlotId)>,
    request_id: Option<u64>,
}

impl PmxLoadRequest {
    /// Creates a request from a file-system path or ZIP archive path.
    ///
    /// A ZIP with no selected entry must contain exactly one PMX file. ZIP
    /// entry discovery remains deferred to the asynchronous loading task so
    /// errors keep their existing notification behavior.
    pub fn from_path(
        path: impl Into<PathBuf>,
        archive_entry: Option<String>,
        existing_slot_ids: Vec<(u32, MaterialSlotId)>,
    ) -> Self {
        Self {
            source: PmxSourceRequest::Path {
                path: path.into(),
                archive_entry,
            },
            existing_slot_ids,
            request_id: None,
        }
    }

    /// Associates an application-owned identifier with this loading request.
    ///
    /// Renderer notifications echo this identifier so a frontend can discard
    /// progress and results from an older request even when it has the same
    /// source path as a newer request. If no identifier is supplied, the
    /// renderer assigns one when the request reaches its worker.
    pub fn with_request_id(mut self, request_id: u64) -> Self {
        self.request_id = Some(request_id);
        self
    }

    /// Returns the optional frontend-owned request identifier.
    pub fn request_id(&self) -> Option<u64> {
        self.request_id
    }

    /// Returns the requested source identity before source resolution.
    pub fn source_identity(&self) -> PmxSourceIdentity {
        match &self.source {
            PmxSourceRequest::Path {
                path,
                archive_entry,
            } => PmxSourceIdentity::new(path.clone(), archive_entry.clone()),
        }
    }

    pub(crate) fn resolve(self) -> Result<ResolvedPmxLoadRequest, String> {
        let source = match self.source {
            PmxSourceRequest::Path {
                path,
                archive_entry,
            } => resolve_path_source(&path, archive_entry.as_deref())?,
        };
        Ok(ResolvedPmxLoadRequest {
            source,
            existing_slot_ids: self.existing_slot_ids,
        })
    }
}

pub(crate) struct ResolvedPmxLoadRequest {
    pub(crate) source: Box<dyn PmxInputSource>,
    pub(crate) existing_slot_ids: Vec<(u32, MaterialSlotId)>,
}

fn resolve_path_source(
    path: &Path,
    archive_entry: Option<&str>,
) -> Result<Box<dyn PmxInputSource>, String> {
    if !is_zip_path(path) {
        return Ok(Box::new(FilePmxSource::new(path.to_path_buf())));
    }

    let entries = discover_pmx_archive_entries(path)?;
    let entry = match archive_entry {
        Some(entry) => normalize_archive_entry(entry)
            .filter(|entry| entries.iter().any(|candidate| candidate == entry))
            .ok_or_else(|| {
                format!(
                    "ZIP archive {} does not contain PMX entry '{}', or the entry path is invalid",
                    path.display(),
                    entry
                )
            })?,
        None => match entries.as_slice() {
            [entry] => entry.clone(),
            [] => {
                return Err(format!(
                    "ZIP archive {} does not contain a PMX file",
                    path.display()
                ));
            }
            _ => {
                return Err(format!(
                    "ZIP archive {} contains multiple PMX files; choose one from the archive",
                    path.display()
                ));
            }
        },
    };

    Ok(Box::new(ZippedPmxSource::new(path.to_path_buf(), entry)))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    use bevy_pmx::{PmxResolvedPath, PmxResolver, PmxSourceLocation};
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::{PmxInputSource, PmxLoadRequest, PmxSourceIdentity, ZippedPmxSource};

    #[test]
    fn identities_normalize_zip_entry_separators() {
        let identity = PmxSourceIdentity::zip("character.zip", r"Model\character.pmx");

        assert_eq!(identity.path().to_str(), Some("character.zip"));
        assert_eq!(identity.archive_entry(), Some("Model/character.pmx"));
    }

    #[test]
    fn requests_keep_path_and_entry_together() {
        let request = PmxLoadRequest::from_path(
            "character.zip",
            Some("Model/character.pmx".to_owned()),
            Vec::new(),
        );

        assert_eq!(
            request.source_identity(),
            PmxSourceIdentity::zip("character.zip", "Model/character.pmx")
        );
    }

    #[test]
    fn zipped_sources_reuse_bevy_texture_resolution_and_byte_reading() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("charme_source_{unique}.zip"));
        let file = fs::File::create(&path).expect("ZIP fixture should be writable");
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("Model/model.pmx", SimpleFileOptions::default())
            .expect("PMX entry should be created");
        writer
            .write_all(b"pmx")
            .expect("PMX entry should be written");
        writer
            .start_file("Model/Texture/Face.PNG", SimpleFileOptions::default())
            .expect("texture entry should be created");
        writer
            .write_all(b"texture")
            .expect("texture entry should be written");
        writer.finish().expect("ZIP fixture should be finished");

        let source = ZippedPmxSource::new(path.clone(), "Model/model.pmx".to_owned());
        let resolved =
            PmxResolver::new().resolve_texture_path(Some(source.bevy_source()), "Texture/face.png");
        assert_eq!(
            resolved,
            PmxResolvedPath::new(
                "Texture/face.png",
                PmxSourceLocation::zip(path.clone(), "Model/Texture/Face.PNG"),
            )
        );
        assert_eq!(
            source
                .read_texture_bytes(&resolved)
                .expect("texture bytes should be readable"),
            b"texture"
        );

        fs::remove_file(path).expect("ZIP fixture should be removable");
    }
}
