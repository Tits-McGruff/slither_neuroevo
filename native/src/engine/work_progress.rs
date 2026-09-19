//! Per-job byte progress readable while a libuv archive task is running.

use std::cell::RefCell;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

thread_local! {
    static CURRENT: RefCell<Option<Arc<AtomicU64>>> = const { RefCell::new(None) };
}

/// Install one counter on the current worker thread for the duration of a job.
pub struct ProgressScope;

impl ProgressScope {
    pub fn enter(counter: Arc<AtomicU64>) -> Self {
        CURRENT.with(|current| {
            let previous = current.replace(Some(counter));
            assert!(
                previous.is_none(),
                "nested archive progress jobs are unsupported"
            );
        });
        Self
    }
}

impl Drop for ProgressScope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(None);
        });
    }
}

/// Count only bytes whose bounded read, write, hash, or codec step completed.
pub fn advance(bytes: usize) {
    if bytes == 0 {
        return;
    }
    CURRENT.with(|current| {
        if let Some(counter) = current.borrow().as_ref() {
            counter.fetch_add(bytes as u64, Ordering::Relaxed);
        }
    });
}

/// Count actual source bytes as callers consume them, never wall-clock ticks.
pub struct ProgressReader<R>(pub R);

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.0.read(bytes)?;
        advance(count);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use super::{advance, ProgressReader, ProgressScope};

    #[test]
    fn completed_reads_and_blocks_advance_only_the_active_job() {
        let first = Arc::new(AtomicU64::new(0));
        let second = Arc::new(AtomicU64::new(0));
        {
            let _scope = ProgressScope::enter(Arc::clone(&first));
            let mut reader = ProgressReader(&b"hello"[..]);
            let mut bytes = [0u8; 8];
            assert_eq!(reader.read(&mut bytes).unwrap(), 5);
            advance(7);
            assert_eq!(first.load(Ordering::Relaxed), 12);
        }
        advance(100);
        {
            let _scope = ProgressScope::enter(Arc::clone(&second));
            advance(3);
        }
        assert_eq!(first.load(Ordering::Relaxed), 12);
        assert_eq!(second.load(Ordering::Relaxed), 3);
    }
}
