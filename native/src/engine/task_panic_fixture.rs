//! One-shot task failures after real output files exist, without production hooks.

use std::cell::Cell;
use std::io;
use std::path::Path;

/// Private preparation boundaries whose unwind cleanup must be exercised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanicPoint {
    CheckpointWritten,
    ExportWritten,
    ImportExtracted,
    ImportFilePublished,
}

thread_local! {
    static ARMED: Cell<Option<PanicPoint>> = const { Cell::new(None) };
    static REACHED: Cell<bool> = const { Cell::new(false) };
    static PUBLICATION_IO_ARMED: Cell<bool> = const { Cell::new(false) };
    static PUBLICATION_IO_REACHED: Cell<bool> = const { Cell::new(false) };
}

/// Inject an ordinary I/O error after one actual new immutable import file rename.
pub(crate) struct PublicationIoErrorInjection;

impl PublicationIoErrorInjection {
    pub(crate) fn arm() -> Self {
        PUBLICATION_IO_ARMED.with(|armed| assert!(!armed.replace(true)));
        PUBLICATION_IO_REACHED.with(|reached| reached.set(false));
        Self
    }

    pub(crate) fn reached(&self) -> bool {
        PUBLICATION_IO_REACHED.with(Cell::get)
    }
}

impl Drop for PublicationIoErrorInjection {
    fn drop(&mut self) {
        PUBLICATION_IO_ARMED.with(|armed| armed.set(false));
        PUBLICATION_IO_REACHED.with(|reached| reached.set(false));
    }
}

/// Exercise late publication failure only after a real new shared file exists.
pub(crate) fn after_import_publication(output: &Path) -> io::Result<()> {
    hit(PanicPoint::ImportFilePublished, output);
    if PUBLICATION_IO_ARMED.with(|armed| armed.replace(false)) {
        let metadata = output.metadata().expect("published output must exist");
        assert!(metadata.is_file() && metadata.len() > 0);
        PUBLICATION_IO_REACHED.with(|reached| reached.set(true));
        return Err(io::Error::other("injected import publication I/O error"));
    }
    Ok(())
}

/// Scope one injection to the current test thread and clear it even on failure.
pub(crate) struct PanicInjection;

impl PanicInjection {
    pub(crate) fn arm(point: PanicPoint) -> Self {
        ARMED.with(|armed| assert!(armed.replace(Some(point)).is_none()));
        REACHED.with(|reached| reached.set(false));
        Self
    }

    pub(crate) fn reached(&self) -> bool {
        REACHED.with(Cell::get)
    }
}

impl Drop for PanicInjection {
    fn drop(&mut self) {
        ARMED.with(|armed| armed.set(None));
        REACHED.with(|reached| reached.set(false));
    }
}

/// Panic only after proving that the real output is a nonempty file.
pub(crate) fn hit(point: PanicPoint, output: &Path) {
    if !ARMED.with(|armed| {
        if armed.get() != Some(point) {
            return false;
        }
        armed.set(None);
        true
    }) {
        return;
    }
    let metadata = output.metadata().expect("operation output must exist");
    assert!(metadata.is_file() && metadata.len() > 0);
    REACHED.with(|reached| reached.set(true));
    panic!("sensitive operation-specific test panic at {point:?}");
}
