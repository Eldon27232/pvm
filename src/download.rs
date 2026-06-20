//! 基于 ureq 的流式下载：写 dest.part，可选 SHA256 校验，成功后原子 rename 到 dest。

use crate::error::{PvmError, Result};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub struct DownloadOpts<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub expect_sha256: Option<&'a str>,
    pub quiet: bool,
}

pub fn download_to(opts: &DownloadOpts) -> Result<()> {
    let resp = ureq::get(opts.url)
        .header("User-Agent", "pvm")
        .call()
        .map_err(|e| PvmError::Http(format!("GET {} 失败: {e}", opts.url)))?;

    let total: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    let mut resp = resp;
    let mut reader = resp.body_mut().as_reader();

    let part = with_added_ext(opts.dest, "part");
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(&part)?;

    let pb = if opts.quiet {
        None
    } else {
        let pb = match total {
            Some(t) => ProgressBar::new(t),
            None => ProgressBar::new_spinner(),
        };
        pb.set_style(
            ProgressStyle::with_template("{bar:40.cyan/blue} {bytes}/{total_bytes} ({eta})")
                .unwrap_or_else(|_| ProgressStyle::default_bar()),
        );
        Some(pb)
    };

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        if opts.expect_sha256.is_some() {
            hasher.update(&buf[..n]);
        }
        downloaded += n as u64;
        if let Some(pb) = &pb {
            pb.set_position(downloaded);
        }
    }
    file.flush()?;
    drop(file);
    if let Some(pb) = &pb {
        pb.finish_and_clear();
    }

    if let Some(expect) = opts.expect_sha256 {
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expect) {
            let _ = std::fs::remove_file(&part);
            return Err(PvmError::Checksum {
                expected: expect.to_string(),
                actual,
            });
        }
    }

    std::fs::rename(&part, opts.dest)?;
    Ok(())
}

/// 对已存在文件做 SHA256 校验。
pub fn verify_sha256(file: &Path, want: &str) -> Result<()> {
    let mut f = File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());
    if actual.eq_ignore_ascii_case(want) {
        Ok(())
    } else {
        Err(PvmError::Checksum {
            expected: want.to_string(),
            actual,
        })
    }
}

/// 在路径末尾追加扩展名（a\b.tar.gz -> a\b.tar.gz.part），不替换原扩展名。
fn with_added_ext(p: &Path, add: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".");
    s.push(add);
    PathBuf::from(s)
}
