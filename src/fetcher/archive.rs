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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_archive_format_from_url() {
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.tar.gz"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.tgz?token=123"),
            Some(ArchiveFormat::TarGz)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.tar.xz"),
            Some(ArchiveFormat::TarXz)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.txz?v=1"),
            Some(ArchiveFormat::TarXz)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.xz"),
            Some(ArchiveFormat::TarXz)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.zip?dl=1"),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.AppImage"),
            Some(ArchiveFormat::AppImage)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool.appimage?foo=bar"),
            Some(ArchiveFormat::AppImage)
        );
        assert_eq!(
            ArchiveFormat::from_url("https://example.com/tool?ext=.tar.gz"),
            None
        );
        assert_eq!(ArchiveFormat::from_url("https://example.com/tool"), None);
    }

    #[test]
    fn test_default_bin_name() {
        assert_eq!(ArchiveFormat::TarGz.default_bin_name("mytool"), "mytool");
        assert_eq!(ArchiveFormat::TarXz.default_bin_name("mytool"), "mytool");
        assert_eq!(ArchiveFormat::Zip.default_bin_name("mytool"), "mytool");
        assert_eq!(ArchiveFormat::AppImage.default_bin_name("mytool"), "AppRun");
    }

    #[test]
    fn test_resolve_relative_bin_path() {
        // Standalone binary (not an archive)
        assert_eq!(
            resolve_relative_bin_path("mytool", "https://example.com/mytool", None).unwrap(),
            PathBuf::from("mytool")
        );

        // Standard archive with default path
        assert_eq!(
            resolve_relative_bin_path("mytool", "https://example.com/mytool.tar.gz", None).unwrap(),
            PathBuf::from("mytool")
        );

        // AppImage with default path
        assert_eq!(
            resolve_relative_bin_path("mytool", "https://example.com/mytool.AppImage", None)
                .unwrap(),
            PathBuf::from("AppRun")
        );

        // Archive with custom relative path
        assert_eq!(
            resolve_relative_bin_path(
                "mytool",
                "https://example.com/mytool.zip",
                Some("bin/custom")
            )
            .unwrap(),
            PathBuf::from("bin/custom")
        );

        // Rejects absolute archive_path
        let err_abs = resolve_relative_bin_path(
            "mytool",
            "https://example.com/mytool.tar.gz",
            Some("/abs/bin"),
        )
        .unwrap_err();
        assert!(err_abs.contains("must be a relative path without '..'"));

        // Rejects parent traversal in archive_path
        let err_dots = resolve_relative_bin_path(
            "mytool",
            "https://example.com/mytool.tar.gz",
            Some("bin/../evil"),
        )
        .unwrap_err();
        assert!(err_dots.contains("must be a relative path without '..'"));
    }

    #[test]
    fn test_extract_tar_gz_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("test.tar.gz");
        {
            let file = File::create(&tar_gz_path).unwrap();
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            let data = b"#!/bin/sh\necho hi\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "bin/mytool", &data[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let unpack_targz = temp_path.join("unpack_targz");
        fs::create_dir_all(&unpack_targz).unwrap();
        extract_and_prepare_archive(
            ArchiveFormat::TarGz,
            File::open(&tar_gz_path).unwrap(),
            temp_path,
            &unpack_targz,
            Path::new("bin/mytool"),
        )
        .unwrap();

        let target_path = unpack_targz.join("bin/mytool");
        assert!(target_path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&target_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "Target is not executable!");
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_extract_tar_gz_escaping_symlink_rejection() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("mytool.tar.gz");
        let tar_gz = File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("/etc/passwd").unwrap();
        builder.append_data(&mut header, "mytool", &[][..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarGz,
            File::open(&tar_gz_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("mytool"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("is a symlink outside of the extracted directory"));
    }

    #[test]
    #[cfg(unix)]
    fn test_extract_tar_gz_internal_symlink_allowed() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("mytool_symlink.tar.gz");
        let tar_gz = File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        let data = b"#!/bin/sh\necho 'internal symlink target'\n";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_size(data.len() as u64);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder.append_data(&mut file_header, "bin/real_tool", &data[..]).unwrap();

        let mut sym_header = tar::Header::new_gnu();
        sym_header.set_size(0);
        sym_header.set_entry_type(tar::EntryType::Symlink);
        sym_header.set_link_name("bin/real_tool").unwrap();
        sym_header.set_cksum();
        builder.append_data(&mut sym_header, "mytool", &[][..]).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarGz,
            File::open(&tar_gz_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("mytool"),
        );

        assert!(result.is_ok(), "Internal symlink should be allowed: {:?}", result.err());
        let target = unpack_dir.join("mytool");
        assert!(target.exists());
        assert!(target.is_symlink());

        use std::os::unix::fs::PermissionsExt;
        let real_meta = fs::metadata(unpack_dir.join("bin/real_tool")).unwrap();
        assert_eq!(real_meta.permissions().mode() & 0o111, 0o111);
    }

    #[test]
    #[cfg(unix)]
    fn test_extract_tar_gz_unpack_filesystem_error() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("conflict.tar.gz");
        let tar_gz = File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);

        // 1. Create a regular file entry named "conflict"
        let mut file_header1 = tar::Header::new_gnu();
        let data1 = b"regular file";
        file_header1.set_size(data1.len() as u64);
        file_header1.set_mode(0o644);
        file_header1.set_cksum();
        builder.append_data(&mut file_header1, "conflict", &data1[..]).unwrap();

        // 2. Create a child file inside "conflict" - OS fails with ENOTDIR (Not a directory)
        let mut file_header2 = tar::Header::new_gnu();
        let data2 = b"blocked content";
        file_header2.set_size(data2.len() as u64);
        file_header2.set_mode(0o644);
        file_header2.set_cksum();
        builder.append_data(&mut file_header2, "conflict/child.txt", &data2[..]).unwrap();

        builder.into_inner().unwrap().finish().unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarGz,
            File::open(&tar_gz_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("conflict"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract tar.gz archive"), "Got: {}", err_msg);
        assert!(err_msg.contains("failed to unpack"), "Got: {}", err_msg);
        assert!(
            err_msg.contains("Not a directory") || err_msg.contains("os error 20"),
            "Underlying filesystem error was not displayed! Got: {}",
            err_msg
        );
    }

    #[test]
    fn test_extract_corrupt_tar_gz() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.tar.gz");
        fs::write(&corrupt_path, b"not a valid tar.gz file").unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarGz,
            File::open(&corrupt_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("corrupt_tool"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract tar.gz archive"), "Got: {}", err_msg);
    }

    #[test]
    fn test_extract_zip_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let zip_path = temp_path.join("test.zip");
        {
            let file = File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o644);
            zip.start_file("bin/", options.clone()).unwrap();
            zip.start_file("bin/myzipbin", options).unwrap();
            zip.write_all(b"#!/bin/sh\necho zip\n").unwrap();
            zip.finish().unwrap();
        }

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        extract_and_prepare_archive(
            ArchiveFormat::Zip,
            File::open(&zip_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("bin/myzipbin"),
        )
        .unwrap();

        let target_path = unpack_dir.join("bin/myzipbin");
        assert!(target_path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&target_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "Target is not executable!");
        }
    }

    #[test]
    fn test_extract_corrupt_zip() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.zip");
        fs::write(&corrupt_path, b"not a valid zip file").unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::Zip,
            File::open(&corrupt_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("corrupt_tool"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to open zip archive"), "Got: {}", err_msg);
    }

    #[test]
    fn test_extract_tar_xz_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            let content = b"echo 'hello tar xz'";
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, "bin/mytool", &content[..]).unwrap();
            builder.finish().unwrap();
        }
        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz_bytes).unwrap();

        let tar_xz_path = temp_path.join("mytool.tar.xz");
        fs::write(&tar_xz_path, &xz_bytes).unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarXz,
            File::open(&tar_xz_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("bin/mytool"),
        );

        assert!(result.is_ok());
        let target_path = unpack_dir.join("bin/mytool");
        assert!(target_path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "File is not executable!");
        }
    }

    #[test]
    fn test_extract_corrupt_xz() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.tar.xz");
        fs::write(&corrupt_path, b"not a valid xz file").unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarXz,
            File::open(&corrupt_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("corrupt_tool"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract xz archive"), "Got: {}", err_msg);
    }

    #[test]
    fn test_extract_corrupt_tar_in_xz() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &b"not a tarball"[..], &mut xz_bytes).unwrap();

        let corrupt_tar_path = temp_path.join("corrupt_tar.tar.xz");
        fs::write(&corrupt_tar_path, &xz_bytes).unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarXz,
            File::open(&corrupt_tar_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("corrupt_tool"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract tar.xz archive"), "Got: {}", err_msg);
    }

    #[test]
    #[cfg(unix)]
    fn test_extract_tar_xz_symlink_rejection() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("/etc/passwd").unwrap();
            builder.append_data(&mut header, "mytool", &[][..]).unwrap();
            builder.finish().unwrap();
        }
        let mut xz_bytes = Vec::new();
        lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz_bytes).unwrap();

        let tar_xz_path = temp_path.join("mytool.tar.xz");
        fs::write(&tar_xz_path, &xz_bytes).unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::TarXz,
            File::open(&tar_xz_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("mytool"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("is a symlink outside of the extracted directory"));
    }

    #[test]
    fn test_extract_appimage_success() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let appimage_path = temp_path.join("mytool.AppImage");
        test_helpers::create_fake_appimage(
            &appimage_path,
            &[
                ("AppRun", b"#!/bin/sh\necho 'hello from AppRun'\n", 0o755),
                ("usr/bin/mytool", b"#!/bin/sh\necho 'hello from usr/bin/mytool'\n", 0o755),
            ],
            &[],
        );

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::AppImage,
            File::open(&appimage_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("AppRun"),
        );

        assert!(result.is_ok(), "Failed to extract AppImage: {:?}", result.err());
        let target_path = unpack_dir.join("AppRun");
        assert!(target_path.exists());
        assert!(unpack_dir.join("usr/bin/mytool").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "AppRun is not executable!");
        }
    }

    #[test]
    fn test_extract_appimage_custom_binary_path_and_symlink() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let appimage_path = temp_path.join("mytool.appimage");
        test_helpers::create_fake_appimage(
            &appimage_path,
            &[
                ("AppRun", b"#!/bin/sh\necho 'hello from AppRun'\n", 0o755),
                ("usr/bin/custom_bin", b"#!/bin/sh\necho 'hello from custom_bin'\n", 0o644),
            ],
            &[("usr/bin/link_to_bin", "custom_bin")],
        );

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::AppImage,
            File::open(&appimage_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("usr/bin/custom_bin"),
        );

        assert!(result.is_ok(), "Failed to extract AppImage: {:?}", result.err());
        let target_path = unpack_dir.join("usr/bin/custom_bin");
        assert!(target_path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&target_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "Target binary is not executable!");
            assert!(unpack_dir.join("usr/bin/link_to_bin").is_symlink());
        }
    }

    #[test]
    fn test_extract_appimage_corrupt_error_propagated() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let corrupt_path = temp_path.join("corrupt.AppImage");
        let data = b"not an ELF file at all";
        fs::write(&corrupt_path, data).unwrap();

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::AppImage,
            File::open(&corrupt_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("AppRun"),
        );

        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("Failed to extract AppImage"), "Got: {}", err_msg);
    }

    #[test]
    #[cfg(unix)]
    fn test_extract_appimage_relative_symlink_apprun_succeeds() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let appimage_path = temp_path.join("symlink_entry.AppImage");
        test_helpers::create_fake_appimage(
            &appimage_path,
            &[("usr/bin/real_bin", b"#!/bin/sh\necho 'real'\n", 0o644)],
            &[("AppRun", "usr/bin/real_bin")],
        );

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let result = extract_and_prepare_archive(
            ArchiveFormat::AppImage,
            File::open(&appimage_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("AppRun"),
        );

        assert!(result.is_ok(), "Expected relative AppRun symlink to succeed, got: {:?}", result.err());
        let target_path = unpack_dir.join("AppRun");
        assert!(target_path.exists());
        assert!(target_path.is_symlink());

        use std::os::unix::fs::PermissionsExt;
        let real_bin_meta = fs::metadata(unpack_dir.join("usr/bin/real_bin")).unwrap();
        assert_eq!(real_bin_meta.permissions().mode() & 0o111, 0o111, "Resolved target must be marked executable");
    }

    #[test]
    fn test_extract_binary_not_found() {
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let tar_gz_path = temp_path.join("test.tar.gz");
        {
            let file = File::create(&tar_gz_path).unwrap();
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            let data = b"#!/bin/sh\necho hi\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "wrong_binary", &data[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }

        let unpack_dir = temp_path.join("unpack");
        fs::create_dir_all(&unpack_dir).unwrap();

        let err = extract_and_prepare_archive(
            ArchiveFormat::TarGz,
            File::open(&tar_gz_path).unwrap(),
            temp_path,
            &unpack_dir,
            Path::new("expected_binary"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("Extracted executable not found at"));
    }
}
