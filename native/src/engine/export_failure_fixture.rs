//! Operation-local archive failures available only in the isolated test addon.

use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Deref;
use std::path::Path;

thread_local! {
    static MODE: Cell<u8> = const { Cell::new(0) };
}

/// Scope a handle's one-shot failure to the libuv thread executing its export.
pub(crate) struct ExportFailureScope;

impl ExportFailureScope {
    pub(crate) fn enter(mode: u8) -> Self {
        MODE.with(|current| assert_eq!(current.replace(mode), 0));
        Self
    }
}

impl Drop for ExportFailureScope {
    fn drop(&mut self) {
        MODE.with(|current| current.set(0));
    }
}

fn take(mode: u8) -> bool {
    MODE.with(|current| {
        if current.get() != mode {
            return false;
        }
        current.set(0);
        true
    })
}

/// Real file writer that can reject only the final two USTAR end blocks.
pub(crate) struct CompletionWriter {
    file: File,
    remaining: Option<u64>,
}

impl CompletionWriter {
    pub(crate) fn new(file: File, expected_bytes: u64) -> Self {
        Self {
            file,
            remaining: take(1).then(|| {
                expected_bytes
                    .checked_sub(1024)
                    .expect("complete USTAR archive must include two end blocks")
            }),
        }
    }
}

impl Write for CompletionWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let limit = match self.remaining {
            Some(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected archive end-block write error",
                ));
            }
            Some(remaining) => bytes.len().min(remaining.try_into().unwrap_or(usize::MAX)),
            None => bytes.len(),
        };
        let written = self.file.write(&bytes[..limit])?;
        if let Some(remaining) = &mut self.remaining {
            *remaining -= written as u64;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Deref for CompletionWriter {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

/// Make the completed operation-owned file fail the actual length check.
pub(crate) fn before_length_check(path: &Path) -> io::Result<()> {
    if take(2) {
        let file = OpenOptions::new().write(true).open(path)?;
        let length = file.metadata()?.len();
        assert!(length > 1024, "real archive contents must exist first");
        file.set_len(length - 1)?;
        file.sync_all()?;
    }
    Ok(())
}

/// Alter one stored role byte before the unchanged full post-write validator.
pub(crate) fn before_validation(path: &Path) -> io::Result<()> {
    if take(3) {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        assert!(file.metadata()?.len() > 1024);
        file.seek(SeekFrom::Start(512))?;
        let mut byte = [0];
        file.read_exact(&mut byte)?;
        byte[0] ^= 1;
        file.seek(SeekFrom::Start(512))?;
        file.write_all(&byte)?;
        file.sync_all()?;
    }
    Ok(())
}
