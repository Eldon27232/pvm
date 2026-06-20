//! 基于 ureq 的流式下载：写 dest.part，可选 SHA256 校验，成功后原子 rename 到 dest。

use crate::error::{PvmError, Result};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct DownloadOpts<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub expect_sha256: Option<&'a str>,
    pub quiet: bool,
}

pub fn download_to(opts: &DownloadOpts) -> Result<()> {
    let resp = crate::net::agent().get(opts.url)
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

/// 多线程分块下载（HTTP Range）。`on_progress(已下载, 总量)` 在协调线程回调，
/// 服务器不支持 Range 或文件较小时自动退回单线程。供 GUI 显示实时进度。
pub fn download_parallel(
    url: &str,
    dest: &Path,
    expect_sha256: Option<&str>,
    threads: usize,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    let (total, supports_range) = probe(url).unwrap_or((0, false));
    let n = threads.clamp(1, 16);
    if !supports_range || total < (1 << 20) || n <= 1 {
        return download_single(url, dest, expect_sha256, total, on_progress);
    }

    let part = with_added_ext(dest, "part");
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = Arc::new(File::create(&part)?);
    file.set_len(total)?;

    let downloaded = Arc::new(AtomicU64::new(0));
    let err: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let chunk = total / n as u64;
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let start = i as u64 * chunk;
        let end = if i == n - 1 { total - 1 } else { (i as u64 + 1) * chunk - 1 };
        let url = url.to_string();
        let file = Arc::clone(&file);
        let downloaded = Arc::clone(&downloaded);
        let err = Arc::clone(&err);
        handles.push(std::thread::spawn(move || {
            if let Err(e) = download_range(&url, start, end, &file, &downloaded) {
                *err.lock().unwrap() = Some(e);
            }
        }));
    }

    // 协调线程轮询累计进度并回调
    loop {
        on_progress(downloaded.load(Ordering::Relaxed), total);
        if handles.iter().all(|h| h.is_finished()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    for h in handles {
        let _ = h.join();
    }
    on_progress(downloaded.load(Ordering::Relaxed), total);

    if let Some(e) = err.lock().unwrap().take() {
        let _ = std::fs::remove_file(&part);
        return Err(PvmError::Http(e));
    }
    if let Some(expect) = expect_sha256 {
        if let Err(e) = verify_sha256(&part, expect) {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// HEAD 探测总大小与 Range 支持。
fn probe(url: &str) -> Result<(u64, bool)> {
    let resp = crate::net::agent().head(url)
        .header("User-Agent", "pvm")
        .call()
        .map_err(|e| PvmError::Http(e.to_string()))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let ranges = resp
        .headers()
        .get("accept-ranges")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("bytes"))
        .unwrap_or(false);
    Ok((total, ranges))
}

fn download_range(
    url: &str,
    start: u64,
    end: u64,
    file: &File,
    downloaded: &AtomicU64,
) -> std::result::Result<(), String> {
    let resp = crate::net::agent().get(url)
        .header("User-Agent", "pvm")
        .header("Range", &format!("bytes={start}-{end}"))
        .call()
        .map_err(|e| e.to_string())?;
    let mut resp = resp;
    let mut reader = resp.body_mut().as_reader();
    let mut offset = start;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let nread = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if nread == 0 {
            break;
        }
        file.seek_write(&buf[..nread], offset)
            .map_err(|e| e.to_string())?;
        offset += nread as u64;
        downloaded.fetch_add(nread as u64, Ordering::Relaxed);
    }
    Ok(())
}

/// 单线程回退下载（带进度回调，无进度条）。
fn download_single(
    url: &str,
    dest: &Path,
    expect_sha256: Option<&str>,
    known_total: u64,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<()> {
    let resp = crate::net::agent().get(url)
        .header("User-Agent", "pvm")
        .call()
        .map_err(|e| PvmError::Http(format!("GET {url} 失败: {e}")))?;
    let total = if known_total > 0 {
        known_total
    } else {
        resp.headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let mut resp = resp;
    let mut reader = resp.body_mut().as_reader();

    let part = with_added_ext(dest, "part");
    if let Some(parent) = part.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = File::create(&part)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let nread = reader.read(&mut buf)?;
        if nread == 0 {
            break;
        }
        file.write_all(&buf[..nread])?;
        if expect_sha256.is_some() {
            hasher.update(&buf[..nread]);
        }
        done += nread as u64;
        on_progress(done, total);
    }
    file.flush()?;
    drop(file);

    if let Some(expect) = expect_sha256 {
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expect) {
            let _ = std::fs::remove_file(&part);
            return Err(PvmError::Checksum {
                expected: expect.to_string(),
                actual,
            });
        }
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}
