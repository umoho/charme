use std::{
    collections::BTreeSet,
    fs::File,
    io,
    path::{Path, PathBuf},
};

use bevy_pmx::ZipNameEncoding;
use zip::{ZipArchive, read::ZipFile};

/// Lists normalized PMX entries contained by a ZIP archive.
///
/// The archive is only inspected; no entries are extracted or modified.
pub fn discover_pmx_archive_entries(path: impl AsRef<Path>) -> Result<Vec<String>, String> {
    let path = path.as_ref();
    let file = File::open(path)
        .map_err(|error| format!("failed to open ZIP archive {}: {error}", path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| format!("failed to read ZIP archive {}: {error}", path.display()))?;
    let mut entries = BTreeSet::new();

    for index in 0..archive.len() {
        let file = archive.by_index_raw(index).map_err(|error| {
            format!(
                "failed to inspect ZIP entry {index} in {}: {error}",
                path.display()
            )
        })?;
        if file.is_dir() {
            continue;
        }
        let Some(entry) = normalized_zip_entry_name(&file) else {
            continue;
        };
        if is_pmx_entry(&entry) {
            entries.insert(entry);
        }
    }

    Ok(entries.into_iter().collect())
}

/// Returns whether a path uses the ZIP model-package extension.
pub(crate) fn is_zip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

pub(crate) fn normalize_archive_entry(value: &str) -> Option<String> {
    let mut components = Vec::new();
    let normalized = value.replace('\\', "/");

    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            "__MACOSX" => return None,
            part if part.starts_with("._") => return None,
            ".." => {
                components.pop()?;
            }
            part => components.push(part),
        }
    }

    Some(components.join("/"))
}

pub(crate) fn archive_root(entry: &str) -> PathBuf {
    Path::new(entry)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn normalized_zip_entry_name<R: io::Read + ?Sized>(file: &ZipFile<'_, R>) -> Option<String> {
    let decoded = ZipNameEncoding::Auto.decode_name(file.name_raw())?;
    normalize_archive_entry(&decoded)
}

fn is_pmx_entry(entry: &str) -> bool {
    Path::new(entry)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pmx"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::{discover_pmx_archive_entries, normalize_archive_entry};

    #[test]
    fn normalizes_archive_paths_without_accepting_noise() {
        assert_eq!(
            normalize_archive_entry(r"Model\Textures\..\character.PMX").as_deref(),
            Some("Model/character.PMX")
        );
        assert_eq!(normalize_archive_entry("__MACOSX/._character.pmx"), None);
        assert_eq!(normalize_archive_entry("../character.pmx"), None);
    }

    #[test]
    fn discovers_sorted_pmx_entries_and_ignores_macos_noise() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("charme_archive_{unique}.zip"));
        let file = fs::File::create(&path).expect("ZIP fixture should be writable");
        let mut writer = ZipWriter::new(file);
        for entry in ["Model/zeta.pmx", "__MACOSX/._noise.pmx", "Model/alpha.PMX"] {
            writer
                .start_file(entry, SimpleFileOptions::default())
                .expect("ZIP entry should be created");
            writer
                .write_all(b"fixture")
                .expect("ZIP entry should be written");
        }
        writer.finish().expect("ZIP fixture should be finished");

        let entries = discover_pmx_archive_entries(&path).expect("ZIP should be inspectable");
        assert_eq!(entries, ["Model/alpha.PMX", "Model/zeta.pmx"]);
        fs::remove_file(path).expect("ZIP fixture should be removable");
    }
}
