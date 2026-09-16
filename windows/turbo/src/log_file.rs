//! Session logs beside the game, with retention performed by the logging worker.
//!
//! Startup writes are synchronous so an early exit still leaves its identity and
//! hook refusals on disk. Runtime writes use the existing worker. Files have no
//! userspace buffer, and cleanup never runs under the loader lock.

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const KEEP: usize = 10;
static SINK: Mutex<Sink> = Mutex::new(Sink::Pending);
static PRUNE_PENDING: AtomicBool = AtomicBool::new(false);

enum Sink {
    Pending,
    Open { file: File, path: PathBuf },
    Stderr,
}

/// Plain-text target for the shared logger's formatter.
pub struct FileSink;

impl Write for FileSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut sink = SINK.lock().unwrap_or_else(PoisonError::into_inner);
        let mut failure = None;
        if matches!(*sink, Sink::Pending) {
            match directory()
                .and_then(|dir| create_session(&dir, SystemTime::now(), std::process::id()))
            {
                Ok((file, path)) => {
                    *sink = Sink::Open { file, path };
                    PRUNE_PENDING.store(true, Ordering::Release);
                }
                Err(error) => {
                    *sink = Sink::Stderr;
                    failure = Some(error);
                }
            }
        }
        let spill = if let Sink::Open { file, .. } = &mut *sink {
            if let Err(error) = file.write_all(bytes) {
                *sink = Sink::Stderr;
                failure = Some(error);
                true
            } else {
                false
            }
        } else {
            true
        };
        drop(sink);
        if spill {
            // Do not use log! here: it would recurse through this same sink.
            let mut stderr = io::stderr().lock();
            if let Some(error) = failure {
                let _ = writeln!(stderr, "[wow] log file: {error}; using stderr");
            }
            let _ = stderr.write_all(bytes);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // File writes already reach the OS. No wait or fsync on game callbacks.
        Ok(())
    }
}

/// Remove old session files once, on the worker after the first file write.
pub fn maintain() {
    if !PRUNE_PENDING.swap(false, Ordering::AcqRel) {
        return;
    }
    let active = {
        let sink = SINK.lock().unwrap_or_else(PoisonError::into_inner);
        match &*sink {
            Sink::Open { path, .. } => Some(path.clone()),
            _ => None,
        }
    };
    if let Some(active) = active {
        prune(&active, KEEP);
    }
}

fn directory() -> io::Result<PathBuf> {
    // Only the test executable accepts an override; the shipped DLL always
    // resolves the game executable, independent of the launcher's working dir.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("WOW_LOG_WORKER_TEST_DIR") {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe()?;
    directory_for(&exe)
}

fn directory_for(exe: &Path) -> io::Result<PathBuf> {
    let parent = exe
        .parent()
        .ok_or_else(|| io::Error::other("game executable has no directory"))?;
    Ok(parent.join("Logs").join("wow_turbo"))
}

fn create_session(dir: &Path, now: SystemTime, pid: u32) -> io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let stamp = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut options = OpenOptions::new();
    options.append(true).create_new(true);
    // Readers may tail the log, but another instance's retention must not
    // delete or open it for writing while this process is still using it.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        options.share_mode(FILE_SHARE_READ);
    }
    // Wine pids repeat across server restarts. A timestamp plus atomic
    // create-new prevents both accidental appends and concurrent collisions.
    for collision in 0..1024 {
        let path = dir.join(format!("wow_turbo-{stamp}-{pid}-{collision}.log"));
        match options.open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many log filename collisions",
    ))
}

fn is_session_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(fields) = name
        .strip_prefix("wow_turbo-")
        .and_then(|s| s.strip_suffix(".log"))
    else {
        return false;
    };
    let mut fields = fields.split('-');
    (0..3).all(|_| {
        fields
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    }) && fields.next().is_none()
}

fn prune(active: &Path, keep: usize) {
    let Some(dir) = active.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut aged: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            // Do not follow links or touch directories/unrelated game logs.
            if path == active || !is_session_file(&path) || !entry.file_type().ok()?.is_file() {
                return None;
            }
            Some((entry.metadata().ok()?.modified().ok()?, path))
        })
        .collect();
    aged.sort();
    let excess = aged.len().saturating_sub(keep.saturating_sub(1));
    for (_, path) in aged.into_iter().take(excess) {
        // Concurrent instances can race here. Windows sharing rules protect
        // live logs; a locked or inaccessible file stays for a later launch.
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
pub mod tests;
