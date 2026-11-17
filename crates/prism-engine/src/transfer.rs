//! Steps shared by HTTP and FTP downloads: choosing the destination, creating the part file,
//! and finalizing (fsync, size check, checksum, atomic rename).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::checksum;
use crate::error::EngineError;
use crate::filename::{self, numbered_name, part_path_for};
use crate::job::{JobShared, StopReason};
use crate::progress::{Completed, DownloadEvent, DownloadState};
use crate::request::{ConflictPolicy, Destination, DownloadRequest, ExpectedChecksum};
use crate::resume::ResumeState;
use crate::segmented::supervisor::Outcome;
use crate::segmented::writer::Writer;
use crate::storage;

/// The chosen paths and the created part file.
pub(crate) struct Prepared {
    pub final_path: PathBuf,
    pub part_path: PathBuf,
    pub file: File,
}

/// Picks the final path for `name` according to the conflict policy, checks free space and
/// creates (and pre-allocates) the part file. With `fixed` the paths of a previous run are
/// reused (restart of a non-resumable download).
pub(crate) fn prepare_destination(
    dest: &Destination,
    suggested_name: &str,
    suffix: &str,
    size: Option<u64>,
    fixed: Option<&Path>,
) -> Result<Prepared, EngineError> {
    let io_err = |e: std::io::Error| EngineError::from_io(&e, size.unwrap_or(0));
    if let Some(final_path) = fixed {
        let dir = final_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        std::fs::create_dir_all(&dir).map_err(io_err)?;
        storage::check_free_space(&dir, size.unwrap_or(0))?;
        let part_path = part_path_for(final_path, suffix);
        let file = storage::create_part_file(&part_path, size, false).map_err(io_err)?;
        return Ok(Prepared {
            final_path: final_path.to_path_buf(),
            part_path,
            file,
        });
    }
    std::fs::create_dir_all(&dest.dir).map_err(io_err)?;
    storage::check_free_space(&dest.dir, size.unwrap_or(0))?;
    let name = match &dest.file_name {
        Some(n) if !n.trim().is_empty() => filename::sanitize(n),
        _ => suggested_name.to_string(),
    };
    match dest.conflict {
        ConflictPolicy::Overwrite => {
            let final_path = dest.dir.join(&name);
            let part_path = part_path_for(&final_path, suffix);
            let file = storage::create_part_file(&part_path, size, false).map_err(io_err)?;
            Ok(Prepared {
                final_path,
                part_path,
                file,
            })
        }
        ConflictPolicy::Fail => {
            let final_path = dest.dir.join(&name);
            let part_path = part_path_for(&final_path, suffix);
            if final_path.exists() {
                return Err(EngineError::FileExists { path: final_path });
            }
            let file = storage::create_part_file(&part_path, size, true).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    EngineError::FileExists {
                        path: part_path.clone(),
                    }
                } else {
                    io_err(e)
                }
            })?;
            Ok(Prepared {
                final_path,
                part_path,
                file,
            })
        }
        ConflictPolicy::Rename => {
            // `create_new` on the part file claims the name atomically even when several
            // downloads pick the same name at the same moment.
            for _ in 0..64 {
                let final_path = filename::unique_path(&dest.dir, &name, suffix, false)
                    .ok_or_else(|| EngineError::FileExists {
                        path: dest.dir.join(&name),
                    })?;
                let part_path = part_path_for(&final_path, suffix);
                match storage::create_part_file(&part_path, size, true) {
                    Ok(file) => {
                        return Ok(Prepared {
                            final_path,
                            part_path,
                            file,
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(io_err(e)),
                }
            }
            Err(EngineError::FileExists {
                path: dest.dir.join(&name),
            })
        }
    }
}

/// Moves the finished part file into place according to the conflict policy. Returns the path
/// actually used.
fn place_file(
    part: &Path,
    final_path: &Path,
    conflict: ConflictPolicy,
) -> Result<PathBuf, EngineError> {
    let io = |e: std::io::Error| EngineError::Io {
        message: format!("cannot rename {}: {e}", part.display()),
    };
    match conflict {
        ConflictPolicy::Overwrite => {
            storage::rename_replace(part, final_path).map_err(io)?;
            Ok(final_path.to_path_buf())
        }
        ConflictPolicy::Fail => {
            if storage::rename_no_replace(part, final_path).map_err(io)? {
                Ok(final_path.to_path_buf())
            } else {
                Err(EngineError::FileExists {
                    path: final_path.to_path_buf(),
                })
            }
        }
        ConflictPolicy::Rename => {
            let dir = final_path.parent().unwrap_or(Path::new(""));
            let name = final_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for n in 0..10_000u32 {
                let candidate = if n == 0 {
                    final_path.to_path_buf()
                } else {
                    dir.join(numbered_name(&name, n))
                };
                if storage::rename_no_replace(part, &candidate).map_err(io)? {
                    return Ok(candidate);
                }
            }
            Err(EngineError::FileExists {
                path: final_path.to_path_buf(),
            })
        }
    }
}

/// Finishes a segmented download: closes the writer (flush + fsync), checks the size,
/// verifies the checksum and renames the part file.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn finalize(
    job: &Arc<JobShared>,
    writer: &Writer,
    part_path: &Path,
    final_path: &Path,
    expected_size: Option<u64>,
    conflict: ConflictPolicy,
    checksum: Option<&ExpectedChecksum>,
    transferred: u64,
    run_started: Instant,
) -> Result<Completed, EngineError> {
    job.set_state(DownloadState::Finalizing);
    writer.close(true).await?;
    let size = std::fs::metadata(part_path)
        .map_err(|e| EngineError::from_io(&e, 0))?
        .len();
    if let Some(expected) = expected_size
        && size != expected
    {
        return Err(EngineError::internal(format!(
            "file has {size} bytes, expected {expected}"
        )));
    }
    let actual = match checksum {
        Some(c) => {
            let (p, algo) = (part_path.to_path_buf(), c.algo);
            let digest = tokio::task::spawn_blocking(move || checksum::hash_file(&p, algo))
                .await
                .map_err(EngineError::internal)?
                .map_err(|e| EngineError::from_io(&e, 0))?;
            Some((checksum::normalize_hex(&c.hex), digest))
        }
        None => None,
    };
    let path = place_file(part_path, final_path, conflict)?;
    {
        let mut m = job.meta();
        m.final_path = Some(path.clone());
        m.total = Some(size);
    }
    if let Some((expected, actual)) = actual
        && expected != actual
    {
        return Err(EngineError::ChecksumMismatch {
            expected,
            actual,
            path,
        });
    }
    Ok(Completed {
        path,
        size,
        checksum_ok: checksum.map(|_| true),
        elapsed: run_started.elapsed(),
        transferred,
        extra_files: Vec::new(),
        warning: None,
    })
}

/// Paths of the part and final file.
pub(crate) struct Paths {
    pub part: std::path::PathBuf,
    pub final_path: std::path::PathBuf,
}

/// Turns the supervisor outcome into the job result (shared with FTP).
pub(crate) async fn end_job(
    job: &Arc<JobShared>,
    writer: &Arc<Writer>,
    paths: &Paths,
    outcome: Result<Outcome, EngineError>,
    req: &DownloadRequest,
    transferred: u64,
    run_started: Instant,
) -> (Result<Completed, EngineError>, Option<ResumeState>) {
    let total = job.meta().total;
    match outcome {
        Ok(Outcome::Completed) => {
            let r = finalize(
                job,
                writer,
                &paths.part,
                &paths.final_path,
                total,
                req.destination.conflict,
                req.checksum.as_ref(),
                transferred,
                run_started,
            )
            .await;
            (r, None)
        }
        Ok(Outcome::Stopped(StopReason::Pause)) => {
            let state = writer
                .flush(true)
                .await
                .ok()
                .and_then(|segs| job.build_resume(segs));
            let closed = writer.close(false).await;
            match closed {
                Ok(()) => (Err(EngineError::Paused), state),
                Err(e) => (Err(e), state),
            }
        }
        Ok(Outcome::Stopped(StopReason::Cancel { delete_file: true })) => {
            // Nothing to keep: no flush or fsync of data that is deleted right away.
            writer.discard().await;
            let _ = std::fs::remove_file(&paths.part);
            (Err(EngineError::Cancelled), None)
        }
        Ok(Outcome::Stopped(StopReason::Cancel { delete_file: false })) => {
            let state = writer
                .flush(true)
                .await
                .ok()
                .and_then(|segs| job.build_resume(segs));
            let _ = writer.close(false).await;
            (Err(EngineError::Cancelled), state)
        }
        Err(e) => {
            if e == EngineError::LinkExpired {
                job.emit(DownloadEvent::LinkExpired);
            }
            // Keep what we have so the download can be resumed (e.g. after refreshing the link).
            let state = writer
                .flush(true)
                .await
                .ok()
                .and_then(|segs| job.build_resume(segs));
            let _ = writer.close(false).await;
            (Err(e), state)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_policies() {
        let dir = tempfile::tempdir().unwrap();
        let dest = |conflict| Destination {
            dir: dir.path().to_path_buf(),
            file_name: None,
            conflict,
        };
        let p = prepare_destination(
            &dest(ConflictPolicy::Rename),
            "a.bin",
            ".prismpart",
            Some(10),
            None,
        )
        .unwrap();
        assert_eq!(p.final_path, dir.path().join("a.bin"));
        assert_eq!(p.file.metadata().unwrap().len(), 10);
        // The part file now claims the name.
        let p2 = prepare_destination(
            &dest(ConflictPolicy::Rename),
            "a.bin",
            ".prismpart",
            None,
            None,
        )
        .unwrap();
        assert_eq!(p2.final_path, dir.path().join("a (1).bin"));
        drop((p.file, p2.file));
        std::fs::write(dir.path().join("b.bin"), b"x").unwrap();
        assert!(matches!(
            prepare_destination(
                &dest(ConflictPolicy::Fail),
                "b.bin",
                ".prismpart",
                None,
                None
            ),
            Err(EngineError::FileExists { .. })
        ));
        let o = prepare_destination(
            &dest(ConflictPolicy::Overwrite),
            "b.bin",
            ".prismpart",
            None,
            None,
        )
        .unwrap();
        drop(o.file);
        assert_eq!(
            place_file(&o.part_path, &o.final_path, ConflictPolicy::Overwrite).unwrap(),
            o.final_path
        );
        // Rename at completion when the final name appeared meanwhile.
        let r = prepare_destination(
            &dest(ConflictPolicy::Rename),
            "c.bin",
            ".prismpart",
            None,
            None,
        )
        .unwrap();
        drop(r.file);
        std::fs::write(dir.path().join("c.bin"), b"late").unwrap();
        assert_eq!(
            place_file(&r.part_path, &r.final_path, ConflictPolicy::Rename).unwrap(),
            dir.path().join("c (1).bin")
        );
        // Explicit file names are sanitized.
        let mut d = dest(ConflictPolicy::Rename);
        d.file_name = Some("x:y?.txt".into());
        let s = prepare_destination(&d, "ignored", ".prismpart", None, None).unwrap();
        assert_eq!(s.final_path, dir.path().join("x_y_.txt"));
    }
}
