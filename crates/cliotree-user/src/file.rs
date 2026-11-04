//! The `.clio` file: the entries of a [`Package`] as a zip, and the file on
//! disk (DESIGN.md, "File format").
//!
//! Writing is deterministic: the entries go in the order the package has them,
//! each with the same fixed time, permissions and compression, and nothing else
//! is recorded, so a workbook that has not changed is saved to the same bytes.
//! The zip is untrusted input when it is read: entry paths are checked, and the
//! number of entries and the sizes are limited, against zip bombs.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use cliotree_calc::{Package, safe_path};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// What a zip may hold when it is read: application preferences (see
/// [`crate::prefs::Preferences`]), with these defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most entries an archive may have.
    pub entries: usize,
    /// The most bytes all entries may have together, uncompressed.
    pub total: u64,
    /// The most bytes one entry may have, uncompressed.
    pub entry: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            entries: 10_000,
            total: 1 << 30,
            entry: 512 << 20,
        }
    }
}

/// The zip of the entries, as bytes.
pub fn write_zip(package: &Package) -> Result<Vec<u8>, String> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6))
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in package.entries() {
        zip.start_file(path, options)
            .map_err(|e| format!("{path}: {e}"))?;
        zip.write_all(bytes).map_err(|e| format!("{path}: {e}"))?;
    }
    let cursor = zip.finish().map_err(|e| e.to_string())?;
    Ok(cursor.into_inner())
}

/// The entries of a zip. Directory entries are skipped; an entry whose path is
/// not safe, an encrypted one, a duplicate, and anything beyond the limits is
/// an error, and nothing is extracted to disk.
pub fn read_zip(bytes: &[u8], limits: &Limits) -> Result<Package, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("this is not a ClioTree workbook (not a zip file: {e})"))?;
    if archive.len() > limits.entries {
        return Err(format!(
            "the file has {} entries, more than the limit of {}",
            archive.len(),
            limits.entries
        ));
    }
    let mut package = Package::new();
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("entry {index}: {e}"))?;
        let name = entry
            .name()
            .map(|n| n.into_owned())
            .map_err(|e| format!("entry {index}: its name is not valid text ({e})"))?;
        if entry.is_dir() {
            continue;
        }
        safe_path(&name)?;
        if entry.encrypted() {
            return Err(format!("{name}: encrypted entries are not supported"));
        }
        if package.get(&name).is_some() {
            return Err(format!("{name}: the archive has this entry twice"));
        }
        if entry.size() > limits.entry {
            return Err(format!(
                "{name}: {} bytes uncompressed, more than the limit of {}",
                entry.size(),
                limits.entry
            ));
        }
        // The size in the header is not to be believed: read at most one byte
        // more than the limit allows and see.
        let room = limits.entry.min(limits.total.saturating_sub(total));
        let mut data = Vec::with_capacity(usize::try_from(entry.size().min(room)).unwrap_or(0));
        entry
            .by_ref()
            .take(room + 1)
            .read_to_end(&mut data)
            .map_err(|e| format!("{name}: {e}"))?;
        if data.len() as u64 > room {
            return Err(format!(
                "{name}: it is larger than the limits allow (zip bomb protection)"
            ));
        }
        total += data.len() as u64;
        package.insert(&name, data)?;
    }
    Ok(package)
}

/// Writes the file so that a crash or a full disk leaves the old one intact:
/// the bytes go to a temporary file in the same directory, which is then
/// renamed over the original.
pub fn save_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let name = path.file_name().map_or_else(
        || "workbook".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// A random id for a workbook. It is not a secret: the trust model needs the
/// content to match as well.
pub fn new_workbook_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let part = |salt: u128| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(nanos ^ salt);
        hasher.finish()
    };
    format!("{:016x}{:016x}", part(1), part(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> Package {
        let mut p = Package::new();
        p.insert("manifest.json", b"{}\n".to_vec()).unwrap();
        p.insert("sheets/A/data.json", b"[1, 2, 3]\n".repeat(100))
            .unwrap();
        p.insert("modules/m.py", b"x = 1\n".to_vec()).unwrap();
        p
    }

    #[test]
    fn a_package_is_written_and_read_back_the_same() {
        let bytes = write_zip(&package()).unwrap();
        assert!(read_zip(&bytes, &Limits::default()).unwrap() == package());
    }

    #[test]
    fn the_same_package_gives_the_same_bytes() {
        assert!(write_zip(&package()).unwrap() == write_zip(&package()).unwrap());
    }

    #[test]
    fn the_first_entry_is_the_manifest_and_nothing_records_a_time() {
        let bytes = write_zip(&package()).unwrap();
        let archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        assert_eq!(archive.name_for_index(0).unwrap().unwrap(), "manifest.json");
        for index in 0..archive.len() {
            let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
            let entry = archive.by_index(index).unwrap();
            let time = entry.last_modified().unwrap();
            assert_eq!(
                (time.year(), time.month(), time.day()),
                (1980, 1, 1),
                "{}",
                entry.name().unwrap()
            );
        }
    }

    #[test]
    fn something_that_is_not_a_zip_is_refused() {
        let error = read_zip(b"this is not a zip", &Limits::default()).unwrap_err();
        assert!(error.contains("not a zip"), "{error}");
    }

    #[test]
    fn too_many_entries_or_too_many_bytes_are_refused() {
        let bytes = write_zip(&package()).unwrap();
        let few = Limits {
            entries: 2,
            ..Limits::default()
        };
        assert!(
            read_zip(&bytes, &few)
                .unwrap_err()
                .contains("more than the limit")
        );
        let small_entry = Limits {
            entry: 100,
            ..Limits::default()
        };
        assert!(read_zip(&bytes, &small_entry).is_err());
        let small_total = Limits {
            total: 500,
            ..Limits::default()
        };
        assert!(read_zip(&bytes, &small_total).is_err());
    }

    #[test]
    fn an_entry_that_reaches_outside_the_archive_is_refused() {
        for name in ["../evil.txt", "/abs.txt", "a/../../b.txt"] {
            let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"x").unwrap();
            let bytes = zip.finish().unwrap().into_inner();
            let error = read_zip(&bytes, &Limits::default()).unwrap_err();
            assert!(error.contains("path"), "{name}: {error}");
        }
    }

    #[test]
    fn a_save_replaces_the_file_and_leaves_no_temporary_one() {
        let directory =
            std::env::temp_dir().join(format!("cliotree-file-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("book.clio");
        save_atomic(&path, b"first").unwrap();
        save_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let names: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["book.clio"]);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
