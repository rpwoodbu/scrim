use appimage::{extract_appimage, format_extract_error};
use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{BufReader, Seek};
use std::path::{Path, PathBuf};
use tar::Archive;
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    TarXz,
    Zip,
    AppImage,
}

impl ArchiveFormat {
    pub fn from_url(url: &str) -> Option<Self> {
        let url_path = url.split('?').next().unwrap_or(url);
        if url_path.ends_with(".tar.gz") || url_path.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if url_path.ends_with(".tar.xz")
            || url_path.ends_with(".txz")
            || url_path.ends_with(".xz")
        {
            Some(Self::TarXz)
        } else if url_path.ends_with(".zip") {
            Some(Self::Zip)
        } else if url_path.ends_with(".AppImage") || url_path.ends_with(".appimage") {
            Some(Self::AppImage)
        } else {
            None
        }
    }

    pub fn default_bin_name(self, tool_name: &str) -> &str {
        match self {
            Self::AppImage => "AppRun",
            Self::TarGz | Self::TarXz | Self::Zip => tool_name,
        }
    }
}

pub fn resolve_relative_bin_path(
    tool_name: &str,
    url: &str,
    archive_path: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(format) = ArchiveFormat::from_url(url) {
        if let Some(bin) = archive_path {
            let bin_path = Path::new(bin);
            if bin_path.is_absolute() || bin.contains("..") {
                return Err(format!(
                    "Invalid archive_path: {} must be a relative path without '..'",
                    bin
                ));
            }
            Ok(PathBuf::from(bin))
        } else {
            Ok(PathBuf::from(format.default_bin_name(tool_name)))
        }
    } else {
        Ok(PathBuf::from(tool_name))
    }
}

pub fn extract_and_prepare_archive(
    format: ArchiveFormat,
    archive_file: File,
    tools_dir: &Path,
    unpack_dir: &Path,
    rel_bin_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match format {
        ArchiveFormat::TarGz => {
            let tar = GzDecoder::new(archive_file);
            let mut archive = Archive::new(tar);
            if let Err(e) = archive.unpack(unpack_dir) {
                return Err(format!(
                    "Failed to extract tar.gz archive: {}",
                    format_extract_error(&e)
                )
                .into());
            }
        }
        ArchiveFormat::TarXz => {
            let mut reader = BufReader::new(archive_file);
            let mut decompressed_tar = tempfile::tempfile_in(tools_dir)?;
            if let Err(e) = lzma_rs::xz_decompress(&mut reader, &mut decompressed_tar) {
                return Err(format!(
                    "Failed to extract xz archive: {}",
                    format_extract_error(&e)
                )
                .into());
            }
            decompressed_tar.rewind()?;
            let mut archive = Archive::new(decompressed_tar);
            if let Err(e) = archive.unpack(unpack_dir) {
                return Err(format!(
                    "Failed to extract tar.xz archive: {}",
                    format_extract_error(&e)
                )
                .into());
            }
        }
        ArchiveFormat::Zip => {
            let mut archive = match ZipArchive::new(archive_file) {
                Ok(a) => a,
                Err(e) => {
                    return Err(
                        format!("Failed to open zip archive: {}", format_extract_error(&e)).into(),
                    );
                }
            };
            if let Err(e) = archive.extract(unpack_dir) {
                return Err(format!(
                    "Failed to extract zip archive: {}",
                    format_extract_error(&e)
                )
                .into());
            }
        }
        ArchiveFormat::AppImage => {
            if let Err(e) = extract_appimage(archive_file, unpack_dir) {
                return Err(format!("Failed to extract AppImage: {}", e).into());
            }
        }
    }

    let temp_target_path = unpack_dir.join(rel_bin_path);
    let symlink_meta = match fs::symlink_metadata(&temp_target_path) {
        Ok(m) => m,
        Err(_) => {
            return Err(format!(
                "Extracted executable not found at {}",
                temp_target_path.display()
            )
            .into());
        }
    };

    let resolved_target_path = if symlink_meta.file_type().is_symlink() {
        let canon_root = unpack_dir.canonicalize()?;
        let canon_target = match temp_target_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                return Err(format!(
                    "Extracted executable {} is a symlink outside of the extracted directory or points to a non-existent target",
                    temp_target_path.display()
                )
                .into());
            }
        };
        if !canon_target.starts_with(&canon_root) {
            return Err(format!(
                "Extracted executable {} is a symlink outside of the extracted directory, which is not allowed",
                temp_target_path.display()
            )
            .into());
        }
        canon_target
    } else {
        temp_target_path
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(&resolved_target_path)?;
        let mut perms = metadata.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&resolved_target_path, perms)?;
    }

    Ok(())
}

