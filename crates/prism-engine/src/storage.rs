//! Part-file creation, pre-allocation and positioned I/O.
//!
//! * Windows: the file is marked sparse (`FSCTL_SET_SPARSE`) before `set_len`, so writes at far
//!   offsets do not trigger NTFS zero-filling from the valid-data length. The flag is cleared
//!   (best effort) only once the file is complete: clearing it on a part file makes NTFS
//!   allocate and zero every range not downloaded yet, in one call (#70: minutes of disk
//!   writes after Pause on a 10 GB file).
//! * Linux: `fallocate` reserves the space (falls back to `set_len` when unsupported).
//! * Elsewhere: `set_len` (sparse on APFS/most Unix file systems).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use crate::error::EngineError;

/// Creates (or truncates) the part file and sizes it. `create_new` fails if it exists.
pub(crate) fn create_part_file(
    path: &Path,
    size: Option<u64>,
    create_new: bool,
) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true);
    if create_new {
        opts.create_new(true);
    } else {
        opts.create(true).truncate(true);
    }
    let file = opts.open(path)?;
    if let Some(size) = size.filter(|s| *s > 0) {
        allocate(&file, size)?;
    }
    Ok(file)
}

/// Opens an existing part file for resuming.
pub(crate) fn open_part_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

fn allocate(file: &File, size: u64) -> io::Result<()> {
    #[cfg(windows)]
    {
        // Best effort: FAT/exFAT do not support sparse files; set_len still works there.
        let _ = set_sparse(file, true);
        file.set_len(size)
    }
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{FallocateFlags, fallocate};
        match fallocate(file, FallocateFlags::empty(), 0, size) {
            Ok(()) => Ok(()),
            Err(e) if e == rustix::io::Errno::NOSPC => Err(io::Error::from(e)),
            // EOPNOTSUPP (tmpfs on old kernels, NFS, FAT …): fall back to a sparse set_len.
            Err(_) => file.set_len(size),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        file.set_len(size)
    }
}

/// Marks (or unmarks) a file as sparse on NTFS/ReFS.
#[cfg(windows)]
pub(crate) fn set_sparse(file: &File, sparse: bool) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{FILE_SET_SPARSE_BUFFER, FSCTL_SET_SPARSE};

    let input = FILE_SET_SPARSE_BUFFER { SetSparse: sparse };
    let mut returned = 0u32;
    // SAFETY: the handle is valid for the lifetime of `file`; the input buffer is a properly
    // initialized FILE_SET_SPARSE_BUFFER of the declared size; no output buffer is used and the
    // call is synchronous (no OVERLAPPED).
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            &input as *const FILE_SET_SPARSE_BUFFER as *const core::ffi::c_void,
            std::mem::size_of::<FILE_SET_SPARSE_BUFFER>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Clears the sparse flag of a complete file (cosmetic; best effort). Never on a part file that
/// is paused or kept: NTFS would write zeros over every range still missing.
pub(crate) fn finish_file(file: &File) {
    #[cfg(windows)]
    {
        let _ = set_sparse(file, false);
    }
    #[cfg(not(windows))]
    {
        let _ = file;
    }
}

/// Writes all of `buf` at `offset` without touching a shared cursor.
#[cfg(windows)]
pub(crate) fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ));
            }
            Ok(n) => {
                buf = &buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Writes all of `buf` at `offset` without touching a shared cursor.
#[cfg(unix)]
pub(crate) fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, buf, offset)
}

/// Reads exactly `buf.len()` bytes at `offset`.
#[cfg(windows)]
pub(crate) fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Reads exactly `buf.len()` bytes at `offset`.
#[cfg(unix)]
pub(crate) fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

/// Fails with [`EngineError::NoSpace`] when the volume holding `dir` has less than `needed`
/// bytes available. Unknown free space (e.g. network shares that do not report it) passes.
pub(crate) fn check_free_space(dir: &Path, needed: u64) -> Result<(), EngineError> {
    if needed == 0 {
        return Ok(());
    }
    match fs4::available_space(dir) {
        Ok(available) if available < needed => Err(EngineError::NoSpace { needed, available }),
        _ => Ok(()),
    }
}

/// Moves `from` to `to`, replacing `to` if it exists (atomic on the same volume).
pub(crate) fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

/// Moves `from` to `to` only if `to` does not exist. Returns `Ok(false)` when it exists.
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let (f, t) = (wide(from), wide(to));
        // SAFETY: both buffers are NUL-terminated UTF-16 strings that outlive the call.
        let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok != 0 {
            return Ok(true);
        }
        let e = io::Error::last_os_error();
        // ERROR_FILE_EXISTS (80) / ERROR_ALREADY_EXISTS (183)
        if matches!(e.raw_os_error(), Some(80) | Some(183)) {
            Ok(false)
        } else {
            Err(e)
        }
    }
    #[cfg(not(windows))]
    {
        // hard_link + remove gives "no replace" semantics on POSIX file systems.
        match std::fs::hard_link(from, to) {
            Ok(()) => {
                std::fs::remove_file(from)?;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            // File systems without hard links: check-then-rename (small race window).
            Err(_) if !to.exists() => std::fs::rename(from, to).map(|_| true),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_write_read_rename() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.prismpart");
        let f = create_part_file(&p, Some(10 << 20), true).unwrap();
        assert_eq!(f.metadata().unwrap().len(), 10 << 20);
        assert!(
            create_part_file(&p, Some(1), true).is_err(),
            "create_new must not clobber"
        );
        write_all_at(&f, b"tail", (10 << 20) - 4).unwrap();
        write_all_at(&f, b"head", 0).unwrap();
        let mut buf = [0u8; 4];
        read_exact_at(&f, &mut buf, (10 << 20) - 4).unwrap();
        assert_eq!(&buf, b"tail");
        read_exact_at(&f, &mut buf, 0).unwrap();
        assert_eq!(&buf, b"head");
        finish_file(&f);
        f.sync_all().unwrap();
        drop(f);
        let fin = dir.path().join("x");
        std::fs::write(&fin, b"old").unwrap();
        assert!(!rename_no_replace(&p, &fin).unwrap());
        rename_replace(&p, &fin).unwrap();
        assert_eq!(std::fs::metadata(&fin).unwrap().len(), 10 << 20);
        let other = dir.path().join("y");
        std::fs::write(dir.path().join("z"), b"1").unwrap();
        assert!(rename_no_replace(&dir.path().join("z"), &other).unwrap());
        assert!(other.exists());
    }

    #[test]
    fn free_space_check() {
        let dir = tempfile::tempdir().unwrap();
        check_free_space(dir.path(), 1).unwrap();
        assert!(matches!(
            check_free_space(dir.path(), u64::MAX),
            Err(EngineError::NoSpace { .. })
        ));
    }
}
