//! One-shot panics after real operation-owned files exist, without production hooks.

use std::cell::Cell;
use std::path::Path;

/// Private preparation boundaries whose unwind cleanup must be exercised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanicPoint {
    CheckpointWritten,
    ExportWritten,
    ImportExtracted,
}

thread_local! {
    static ARMED: Cell<Option<PanicPoint>> = const { Cell::new(None) };
    static REACHED: Cell<bool> = const { Cell::new(false) };
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

/// Panic only after proving that the real private output is a nonempty file.
pub(crate) fn hit(point: PanicPoint, private_output: &Path) {
    if !ARMED.with(|armed| {
        if armed.get() != Some(point) {
            return false;
        }
        armed.set(None);
        true
    }) {
        return;
    }
    let metadata = private_output
        .metadata()
        .expect("private output must exist");
    assert!(metadata.is_file() && metadata.len() > 0);
    REACHED.with(|reached| reached.set(true));
    panic!("sensitive operation-specific test panic at {point:?}");
}
