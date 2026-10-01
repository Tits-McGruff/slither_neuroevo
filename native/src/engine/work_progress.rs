//! Per-job byte progress readable while a libuv archive task is running.

use std::cell::RefCell;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Hard diagnostic record cap, independent of archive bytes and numeric block count.
pub const MAX_ARCHIVE_PHASE_INTERVALS: usize = 4096;

/// Fixed internal stages; no owner data or filenames enter the diagnostic trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchivePhase {
    Export,
    ExportSourcePopulation,
    Import,
    Validation,
    CheckpointRestore,
    NumericEncode,
    NumericDecode,
    CandidateConstruction,
    ManagedPublication,
    TemporaryFileWrite,
    LegacyDecode,
}

impl ArchivePhase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Export => "export",
            Self::ExportSourcePopulation => "export-source-population",
            Self::Import => "import",
            Self::Validation => "validation",
            Self::CheckpointRestore => "checkpoint-restore",
            Self::NumericEncode => "numeric-encode",
            Self::NumericDecode => "numeric-decode",
            Self::CandidateConstruction => "candidate-construction",
            Self::ManagedPublication => "managed-publication",
            Self::TemporaryFileWrite => "temporary-file-write",
            Self::LegacyDecode => "legacy-decode",
        }
    }
}

/// One nested interval relative to this job's monotonic clock.
#[derive(Clone, Debug)]
pub struct ArchivePhaseInterval {
    pub phase: ArchivePhase,
    pub started_micros: u64,
    pub finished_micros: Option<u64>,
}

/// Opt-in bounded timing history, readable during work and retained after completion.
pub struct ArchivePhaseTrace {
    started: Instant,
    intervals: Mutex<Vec<ArchivePhaseInterval>>,
    truncated: AtomicBool,
}

impl Default for ArchivePhaseTrace {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchivePhaseTrace {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            intervals: Mutex::new(Vec::new()),
            truncated: AtomicBool::new(false),
        }
    }

    pub fn snapshot(&self) -> (u64, bool, Vec<ArchivePhaseInterval>) {
        let intervals = self
            .intervals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (
            self.elapsed_micros(),
            self.truncated.load(Ordering::Relaxed),
            intervals.clone(),
        )
    }

    fn elapsed_micros(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    fn begin(&self, phase: ArchivePhase) -> Option<usize> {
        let mut intervals = self
            .intervals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if intervals.len() == MAX_ARCHIVE_PHASE_INTERVALS {
            self.truncated.store(true, Ordering::Relaxed);
            return None;
        }
        let index = intervals.len();
        intervals.push(ArchivePhaseInterval {
            phase,
            started_micros: self.elapsed_micros(),
            finished_micros: None,
        });
        Some(index)
    }

    fn finish(&self, index: usize) {
        let mut intervals = self
            .intervals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        intervals[index].finished_micros = Some(self.elapsed_micros());
    }
}

/// Thread-owned progress context; ordinary coordinator scopes carry no archive trace.
struct ProgressContext {
    counter: Arc<AtomicU64>,
    trace: Option<Arc<ArchivePhaseTrace>>,
}

thread_local! {
    static CURRENT: RefCell<Option<ProgressContext>> = const { RefCell::new(None) };
}

/// Install one counter on the current worker thread for the duration of a job.
pub struct ProgressScope;

impl ProgressScope {
    pub fn enter(counter: Arc<AtomicU64>) -> Self {
        Self::enter_with_trace(counter, None)
    }

    pub fn enter_with_trace(
        counter: Arc<AtomicU64>,
        trace: Option<Arc<ArchivePhaseTrace>>,
    ) -> Self {
        CURRENT.with(|current| {
            let previous = current.replace(Some(ProgressContext { counter, trace }));
            assert!(
                previous.is_none(),
                "nested archive progress jobs are unsupported"
            );
        });
        Self
    }
}

/// Close even failed or panicking stages without retaining population data.
pub struct ArchivePhaseScope(Option<(Arc<ArchivePhaseTrace>, usize)>);

impl ArchivePhaseScope {
    pub fn enter(phase: ArchivePhase) -> Self {
        let trace = CURRENT.with(|current| {
            current
                .borrow()
                .as_ref()
                .and_then(|context| context.trace.clone())
        });
        Self(trace.and_then(|trace| trace.begin(phase).map(|index| (trace, index))))
    }
}

impl Drop for ArchivePhaseScope {
    fn drop(&mut self) {
        if let Some((trace, index)) = &self.0 {
            trace.finish(*index);
        }
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
            counter.counter.fetch_add(bytes as u64, Ordering::Relaxed);
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

    use super::{
        advance, ArchivePhase, ArchivePhaseScope, ArchivePhaseTrace, ProgressReader, ProgressScope,
        MAX_ARCHIVE_PHASE_INTERVALS,
    };

    /// Real nested scopes remain attributable while running and after their guards drop.
    #[test]
    fn nested_archive_phases_close_in_order_without_changing_byte_progress() {
        let trace = Arc::new(ArchivePhaseTrace::new());
        let counter = Arc::new(AtomicU64::new(0));
        {
            let _job =
                ProgressScope::enter_with_trace(Arc::clone(&counter), Some(Arc::clone(&trace)));
            let _outer = ArchivePhaseScope::enter(ArchivePhase::Export);
            {
                let _inner = ArchivePhaseScope::enter(ArchivePhase::NumericDecode);
                advance(17);
                let (_, truncated, intervals) = trace.snapshot();
                assert!(!truncated);
                assert_eq!(intervals.len(), 2);
                assert!(intervals
                    .iter()
                    .all(|interval| interval.finished_micros.is_none()));
            }
            assert!(trace.snapshot().2[1].finished_micros.is_some());
        }
        let (elapsed, truncated, intervals) = trace.snapshot();
        assert!(!truncated);
        assert_eq!(intervals[0].phase, ArchivePhase::Export);
        assert_eq!(intervals[1].phase, ArchivePhase::NumericDecode);
        assert!(intervals[0].started_micros <= intervals[1].started_micros);
        assert!(intervals[1].finished_micros.unwrap() <= intervals[0].finished_micros.unwrap());
        assert!(intervals[0].finished_micros.unwrap() <= elapsed);
        assert_eq!(counter.load(Ordering::Relaxed), 17);
    }

    /// Long histories hit a fixed record cap and explicitly report incomplete timings.
    #[test]
    fn archive_phase_overflow_is_bounded_and_does_not_interrupt_work() {
        let trace = Arc::new(ArchivePhaseTrace::new());
        let counter = Arc::new(AtomicU64::new(0));
        let _job = ProgressScope::enter_with_trace(Arc::clone(&counter), Some(Arc::clone(&trace)));
        for _ in 0..MAX_ARCHIVE_PHASE_INTERVALS + 3 {
            let _phase = ArchivePhaseScope::enter(ArchivePhase::NumericEncode);
            advance(1);
        }
        let (_, truncated, intervals) = trace.snapshot();
        assert!(truncated);
        assert_eq!(intervals.len(), MAX_ARCHIVE_PHASE_INTERVALS);
        assert!(intervals
            .iter()
            .all(|interval| interval.finished_micros.is_some()));
        assert_eq!(
            counter.load(Ordering::Relaxed),
            (MAX_ARCHIVE_PHASE_INTERVALS + 3) as u64
        );
    }

    /// Failed jobs close their active intervals and cannot attach timing to a later job.
    #[test]
    fn archive_phase_unwind_closes_intervals_and_clears_the_thread_context() {
        let trace = Arc::new(ArchivePhaseTrace::new());
        let counter = Arc::new(AtomicU64::new(0));
        let result = std::panic::catch_unwind(|| {
            let _job =
                ProgressScope::enter_with_trace(Arc::clone(&counter), Some(Arc::clone(&trace)));
            let _phase = ArchivePhaseScope::enter(ArchivePhase::Validation);
            panic!("injected phase failure");
        });
        assert!(result.is_err());
        assert!(trace.snapshot().2[0].finished_micros.is_some());
        let _inactive = ArchivePhaseScope::enter(ArchivePhase::Import);
        advance(100);
        assert_eq!(trace.snapshot().2.len(), 1);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        let next = Arc::new(AtomicU64::new(0));
        let _job = ProgressScope::enter(Arc::clone(&next));
        advance(7);
        assert_eq!(next.load(Ordering::Relaxed), 7);
    }

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
