//! The per-download writer thread.
//!
//! Workers hand received chunks to a dedicated OS thread over a channel. The thread coalesces
//! each worker's contiguous chunks into buffers of `write_buffer` bytes and issues positioned
//! writes; it is the only place that writes the part file and advances the segments' durable
//! (`written`) frontier. Memory is bounded by an engine-wide byte budget (a semaphore): a
//! worker reserves budget before handing over a chunk and the permit is released once the
//! bytes hit the OS. When the budget is exhausted, workers stop reading their sockets and TCP
//! flow control slows the servers down.
//!
//! `flush(sync = true)` is the checkpoint barrier: it writes all buffered data, `fsync`s and
//! returns the segment states captured *after* the sync, so a persisted state never claims
//! bytes that are not durably on disk.

use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

use super::table::{SegmentTable, WorkerId};
use crate::error::EngineError;
use crate::resume::SegmentState;
use crate::storage;

/// Pending buffers are flushed when no new data arrived for this long.
const IDLE_FLUSH: Duration = Duration::from_millis(200);

enum Msg {
    Chunk {
        worker: WorkerId,
        seg: usize,
        offset: u64,
        data: Bytes,
        permit: OwnedSemaphorePermit,
    },
    Flush {
        sync: bool,
        reply: oneshot::Sender<Result<Vec<SegmentState>, EngineError>>,
    },
    Close {
        /// Every byte is written: the file can stop being sparse.
        complete: bool,
        reply: oneshot::Sender<Result<(), EngineError>>,
    },
    /// The file is about to be deleted: drop what's buffered and close it without syncing.
    Discard { reply: oneshot::Sender<()> },
}

struct Pending {
    worker: WorkerId,
    seg: usize,
    offset: u64,
    data: BytesMut,
    permit: Option<OwnedSemaphorePermit>,
}

/// Handle to a download's writer thread.
pub(crate) struct Writer {
    tx: Mutex<Option<mpsc::Sender<Msg>>>,
    budget: Arc<Semaphore>,
    budget_total: usize,
    failed: Arc<AtomicBool>,
    error: Arc<Mutex<Option<EngineError>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Writer {
    /// Starts the writer thread for `file`.
    pub(crate) fn spawn(
        file: File,
        table: Arc<SegmentTable>,
        budget: Arc<Semaphore>,
        budget_total: usize,
        write_buffer: usize,
    ) -> std::io::Result<Arc<Writer>> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let failed = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let thread = {
            let (failed, error, budget) = (failed.clone(), error.clone(), budget.clone());
            std::thread::Builder::new()
                .name("prism-writer".into())
                .spawn(move || {
                    WriterThread {
                        file,
                        table,
                        budget,
                        budget_total,
                        write_buffer,
                        pending: Vec::new(),
                        failed,
                        error,
                    }
                    .run(rx)
                })?
        };
        Ok(Arc::new(Writer {
            tx: Mutex::new(Some(tx)),
            budget,
            budget_total,
            failed,
            error,
            thread: Mutex::new(Some(thread)),
        }))
    }

    /// The first write error, if any.
    pub(crate) fn error(&self) -> Option<EngineError> {
        if self.failed.load(Ordering::Acquire) {
            self.error.lock().expect("writer error lock").clone()
        } else {
            None
        }
    }

    fn sender(&self) -> Result<mpsc::Sender<Msg>, EngineError> {
        self.tx
            .lock()
            .expect("writer tx lock")
            .clone()
            .ok_or_else(|| EngineError::internal("writer already closed"))
    }

    /// Reserves memory budget for a chunk of `n` bytes (waits under memory pressure).
    pub(crate) async fn reserve(&self, n: usize) -> Result<OwnedSemaphorePermit, EngineError> {
        let n = n.clamp(1, self.budget_total) as u32;
        self.budget
            .clone()
            .acquire_many_owned(n)
            .await
            .map_err(|_| EngineError::internal("budget closed"))
    }

    /// Hands a chunk to the writer thread (never blocks).
    pub(crate) fn submit(
        &self,
        worker: WorkerId,
        seg: usize,
        offset: u64,
        data: Bytes,
        permit: OwnedSemaphorePermit,
    ) -> Result<(), EngineError> {
        if let Some(e) = self.error() {
            return Err(e);
        }
        self.sender()?
            .send(Msg::Chunk {
                worker,
                seg,
                offset,
                data,
                permit,
            })
            .map_err(|_| {
                self.error()
                    .unwrap_or_else(|| EngineError::internal("writer thread stopped"))
            })
    }

    /// Writes all buffered data (and `fsync`s with `sync`); returns the durable segment states
    /// captured after the flush.
    pub(crate) async fn flush(&self, sync: bool) -> Result<Vec<SegmentState>, EngineError> {
        let (reply, rx) = oneshot::channel();
        self.sender()?
            .send(Msg::Flush { sync, reply })
            .map_err(|_| self.closed_error())?;
        rx.await.map_err(|_| self.closed_error())?
    }

    /// Flushes, fsyncs, closes the file and joins the thread. Further submits fail. `complete`
    /// is for a finished download only (see [`storage::finish_file`]).
    pub(crate) async fn close(&self, complete: bool) -> Result<(), EngineError> {
        let Ok(tx) = self.sender() else {
            return self.error().map_or(Ok(()), Err);
        };
        let (reply, rx) = oneshot::channel();
        let sent = tx.send(Msg::Close { complete, reply }).is_ok();
        drop(tx);
        self.tx.lock().expect("writer tx lock").take();
        let result = if sent {
            rx.await.unwrap_or_else(|_| Err(self.closed_error()))
        } else {
            Err(self.closed_error())
        };
        let handle = self.thread.lock().expect("writer thread lock").take();
        if let Some(h) = handle {
            let _ = tokio::task::spawn_blocking(move || h.join()).await;
        }
        result
    }

    /// Closes the file without writing what's buffered or syncing it, and joins the thread: for a
    /// download that is cancelled and deleted. Flushing and fsyncing data that is deleted right
    /// after only made the cancel slow (seconds on a busy disk with a virus scanner).
    pub(crate) async fn discard(&self) {
        let Ok(tx) = self.sender() else {
            return;
        };
        let (reply, rx) = oneshot::channel();
        let sent = tx.send(Msg::Discard { reply }).is_ok();
        drop(tx);
        self.tx.lock().expect("writer tx lock").take();
        if sent {
            let _ = rx.await;
        }
        let handle = self.thread.lock().expect("writer thread lock").take();
        if let Some(h) = handle {
            let _ = tokio::task::spawn_blocking(move || h.join()).await;
        }
    }

    fn closed_error(&self) -> EngineError {
        self.error()
            .unwrap_or_else(|| EngineError::internal("writer thread stopped"))
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // Closing the channel makes the thread flush and exit on its own.
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
    }
}

struct WriterThread {
    file: File,
    table: Arc<SegmentTable>,
    budget: Arc<Semaphore>,
    budget_total: usize,
    write_buffer: usize,
    pending: Vec<Pending>,
    failed: Arc<AtomicBool>,
    error: Arc<Mutex<Option<EngineError>>>,
}

impl WriterThread {
    fn run(mut self, rx: mpsc::Receiver<Msg>) {
        loop {
            let msg = if self.pending.is_empty() {
                rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
            } else {
                rx.recv_timeout(IDLE_FLUSH)
            };
            match msg {
                Ok(Msg::Chunk {
                    worker,
                    seg,
                    offset,
                    data,
                    permit,
                }) => {
                    if self.failed.load(Ordering::Acquire) {
                        continue;
                    }
                    self.add(worker, seg, offset, data, permit);
                    // Under memory pressure, release everything we hold.
                    if self.budget.available_permits() < self.budget_total / 4 {
                        self.flush_all();
                    }
                }
                Ok(Msg::Flush { sync, reply }) => {
                    self.flush_all();
                    let r = self.check().and_then(|()| {
                        if sync {
                            self.file.sync_data().map_err(|e| self.fail(&e))?;
                        }
                        Ok(self.table.durable_states())
                    });
                    let _ = reply.send(r);
                }
                Ok(Msg::Close { complete, reply }) => {
                    self.flush_all();
                    let r = self.check().and_then(|()| {
                        self.file.sync_all().map_err(|e| self.fail(&e))?;
                        if complete {
                            storage::finish_file(&self.file);
                        }
                        Ok(())
                    });
                    let _ = reply.send(r);
                    return;
                }
                Ok(Msg::Discard { reply }) => {
                    // The buffers' budget permits are returned as they drop; the file closes when
                    // the thread ends.
                    self.pending.clear();
                    let _ = reply.send(());
                    return;
                }
                Err(RecvTimeoutError::Timeout) => self.flush_all(),
                Err(RecvTimeoutError::Disconnected) => {
                    self.flush_all();
                    return;
                }
            }
        }
    }

    fn check(&self) -> Result<(), EngineError> {
        if self.failed.load(Ordering::Acquire) {
            Err(self
                .error
                .lock()
                .expect("writer error lock")
                .clone()
                .unwrap_or_else(|| EngineError::internal("write failed")))
        } else {
            Ok(())
        }
    }

    fn fail(&self, e: &std::io::Error) -> EngineError {
        let err = EngineError::from_io(e, 0);
        let mut slot = self.error.lock().expect("writer error lock");
        if slot.is_none() {
            *slot = Some(err.clone());
        }
        self.failed.store(true, Ordering::Release);
        err
    }

    fn add(
        &mut self,
        worker: WorkerId,
        seg: usize,
        offset: u64,
        data: Bytes,
        permit: OwnedSemaphorePermit,
    ) {
        if let Some(i) = self.pending.iter().position(|p| p.worker == worker) {
            let p = &mut self.pending[i];
            if p.seg == seg && p.offset + p.data.len() as u64 == offset {
                p.data.extend_from_slice(&data);
                match &mut p.permit {
                    Some(held) => held.merge(permit),
                    None => p.permit = Some(permit),
                }
                if p.data.len() >= self.write_buffer {
                    let p = self.pending.swap_remove(i);
                    self.write(p);
                }
                return;
            }
            // Not contiguous (new segment or new position): write the old buffer first.
            let p = self.pending.swap_remove(i);
            self.write(p);
        }
        let mut buf = BytesMut::with_capacity(self.write_buffer.max(data.len()));
        buf.extend_from_slice(&data);
        self.pending.push(Pending {
            worker,
            seg,
            offset,
            data: buf,
            permit: Some(permit),
        });
    }

    fn flush_all(&mut self) {
        for p in std::mem::take(&mut self.pending) {
            self.write(p);
        }
    }

    fn write(&mut self, p: Pending) {
        if self.failed.load(Ordering::Acquire) || p.data.is_empty() {
            return;
        }
        match storage::write_all_at(&self.file, &p.data, p.offset) {
            Ok(()) => self.table.on_flushed(p.seg, p.offset, p.data.len() as u64),
            Err(e) => {
                self.fail(&e);
                self.pending.clear();
            }
        }
        // `p.permit` is dropped here, returning the budget.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn coalesces_writes_and_reports_durable_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.prismpart");
        let total = 3 << 20;
        let file = storage::create_part_file(&path, Some(total), true).unwrap();
        let table = Arc::new(SegmentTable::new(Some(total), 1 << 20, true));
        let budget = Arc::new(Semaphore::new(8 << 20));
        let w = Writer::spawn(file, table.clone(), budget.clone(), 8 << 20, 256 * 1024).unwrap();
        let (a, _) = table.acquire(0, false).unwrap();
        let data: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        let mut off = 0u64;
        for chunk in data.chunks(10_000) {
            let permit = w.reserve(chunk.len()).await.unwrap();
            let adv = table.advance(a.seg, 0, off, chunk.len() as u64);
            w.submit(
                0,
                a.seg,
                off,
                Bytes::copy_from_slice(&chunk[..adv.accept as usize]),
                permit,
            )
            .unwrap();
            off += adv.accept;
        }
        let states = w.flush(true).await.unwrap();
        assert_eq!(states[0].pos, total);
        assert_eq!(budget.available_permits(), 8 << 20, "budget fully returned");
        w.close(true).await.unwrap();
        assert!(
            w.submit(
                0,
                0,
                0,
                Bytes::new(),
                budget.clone().acquire_owned().await.unwrap()
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    /// A cancelled-and-deleted download: buffered data is dropped, the budget comes back, and the
    /// file is closed (Windows can delete it right away).
    #[tokio::test]
    async fn discard_drops_buffers_and_closes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.prismpart");
        let total = 1 << 20;
        let file = storage::create_part_file(&path, Some(total), true).unwrap();
        let table = Arc::new(SegmentTable::new(Some(total), 1 << 20, true));
        let budget = Arc::new(Semaphore::new(4 << 20));
        let w = Writer::spawn(file, table.clone(), budget.clone(), 4 << 20, 1 << 20).unwrap();
        let (a, _) = table.acquire(0, false).unwrap();
        let permit = w.reserve(64 * 1024).await.unwrap();
        w.submit(0, a.seg, 0, Bytes::from(vec![7u8; 64 * 1024]), permit)
            .unwrap();
        w.discard().await;
        assert_eq!(budget.available_permits(), 4 << 20, "budget fully returned");
        assert!(
            w.submit(
                0,
                0,
                0,
                Bytes::new(),
                budget.clone().acquire_owned().await.unwrap()
            )
            .is_err()
        );
        std::fs::remove_file(&path).expect("the file is closed and can be deleted");
        // Discarding twice (or after close) is harmless.
        w.discard().await;
    }

    /// #70: a paused (or failed, or kept) download stays sparse. Clearing the flag made NTFS
    /// write zeros over everything not downloaded yet: minutes of disk writes on a big file.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_paused_part_file_stays_sparse_and_a_finished_one_does_not() {
        use std::os::windows::fs::MetadataExt;
        const SPARSE: u32 = 0x200; // FILE_ATTRIBUTE_SPARSE_FILE
        let sparse =
            |p: &std::path::Path| std::fs::metadata(p).unwrap().file_attributes() & SPARSE != 0;
        let dir = tempfile::tempdir().unwrap();

        let path = dir.path().join("p.prismpart");
        let total = 4u64 << 30;
        let file = storage::create_part_file(&path, Some(total), true).unwrap();
        if !sparse(&path) {
            return; // A file system without sparse files (FAT): nothing to check.
        }
        let table = Arc::new(SegmentTable::new(Some(total), 1 << 20, true));
        let budget = Arc::new(Semaphore::new(4 << 20));
        let w = Writer::spawn(file, table.clone(), budget, 4 << 20, 1 << 20).unwrap();
        let (a, _) = table.acquire(0, false).unwrap();
        let permit = w.reserve(64 * 1024).await.unwrap();
        w.submit(0, a.seg, 0, Bytes::from(vec![7u8; 64 * 1024]), permit)
            .unwrap();
        w.flush(true).await.unwrap();
        let started = std::time::Instant::now();
        w.close(false).await.unwrap();
        assert!(sparse(&path), "a paused part file keeps its sparse flag");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "closing wrote no zeros"
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), total);

        let done = dir.path().join("d.prismpart");
        let total = 1u64 << 20;
        let file = storage::create_part_file(&done, Some(total), true).unwrap();
        let table = Arc::new(SegmentTable::new(Some(total), 1 << 20, true));
        let budget = Arc::new(Semaphore::new(4 << 20));
        let w = Writer::spawn(file, table.clone(), budget, 4 << 20, 1 << 20).unwrap();
        let (a, _) = table.acquire(0, false).unwrap();
        let permit = w.reserve(total as usize).await.unwrap();
        w.submit(0, a.seg, 0, Bytes::from(vec![1u8; total as usize]), permit)
            .unwrap();
        w.close(true).await.unwrap();
        assert!(!sparse(&done), "a finished file is no longer sparse");
    }
}
