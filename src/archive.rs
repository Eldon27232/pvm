//! 归档解压，按类型分派；对每个 entry 做路径穿越校验（拒绝 .. 与绝对路径）。
//! 调用方保证 dest 为临时目录，外层做原子 rename。

use crate::error::{PvmError, Result};
use std::fs::File;
use std::io::BufReader;
use std::path::{Component, Path, PathBuf};

pub enum ArchiveKind {
    TarGz,
    TarZst,
    Zip,
}

pub fn extract(archive: &Path, dest: &Path, kind: ArchiveKind) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    match kind {
        ArchiveKind::TarGz => {
            let f = File::open(archive)?;
            let gz = flate2::read::GzDecoder::new(BufReader::new(f));
            unpack_tar(gz, dest)
        }
        ArchiveKind::TarZst => extract_tar_zst(archive, dest),
        ArchiveKind::Zip => extract_zip(archive, dest),
    }
}

fn unpack_tar<R: std::io::Read>(reader: R, dest: &Path) -> Result<()> {
    let mut ar = tar::Archive::new(reader);
    let entries = ar.entries().map_err(|e| PvmError::Archive(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| PvmError::Archive(e.to_string()))?;
        let path = entry.path().map_err(|e| PvmError::Archive(e.to_string()))?;
        let safe = sanitize(&path)
            .ok_or_else(|| PvmError::Archive(format!("非法归档路径: {}", path.display())))?;
        let out = dest.join(&safe);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry
            .unpack(&out)
            .map_err(|e| PvmError::Archive(e.to_string()))?;
    }
    Ok(())
}

#[cfg(feature = "zstd-full")]
fn extract_tar_zst(archive: &Path, dest: &Path) -> Result<()> {
    let f = File::open(archive)?;
    let dec =
        zstd::Decoder::new(BufReader::new(f)).map_err(|e| PvmError::Archive(e.to_string()))?;
    unpack_tar(dec, dest)
}

#[cfg(not(feature = "zstd-full"))]
fn extract_tar_zst(_archive: &Path, _dest: &Path) -> Result<()> {
    Err(PvmError::Archive(
        "本二进制未启用 zstd-full feature，无法解压 .tar.zst（--flavor full）".into(),
    ))
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let f = File::open(archive)?;
    let mut zip =
        zip::ZipArchive::new(BufReader::new(f)).map_err(|e| PvmError::Archive(e.to_string()))?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| PvmError::Archive(e.to_string()))?;
        let name = match file.enclosed_name() {
            Some(p) => p,
            None => return Err(PvmError::Archive(format!("非法 zip 路径: {}", file.name()))),
        };
        let out = dest.join(name);
        if file.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut o = File::create(&out)?;
            std::io::copy(&mut file, &mut o)?;
        }
    }
    Ok(())
}

/// 归一化归档内路径：剥离 . 组件，拒绝 .. 与绝对/盘符路径。
fn sanitize(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}
