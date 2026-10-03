//! Per-job byte progress readable while a libuv archive task is running.

use std::cell::RefCell;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Hard diagnostic record cap, independent of archive bytes and numeric block count.
pub const MAX_ARCHIVE_PHASE_INTERVALS: usize = 4096;

/// Requested diagnostic cadence; observed peaks remain sampled, not exhaustive.
pub const ARCHIVE_RSS_SAMPLE_INTERVAL_MICROS: u64 = 2000;

/// Query only this process; errors stay diagnostic and never change archive work.
#[cfg(target_os = "linux")]
fn resident_bytes() -> Option<u64> {
    let mut status = String::new();
    std::fs::File::open("/proc/self/status")
        .ok()?
        .take(64 * 1024)
        .read_to_string(&mut status)
        .ok()?;
    let mut fields = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace();
    let kib = fields.next()?.parse::<u64>().ok()?;
    if fields.next()? != "kB" {
        return None;
    }
    kib.checked_mul(1024)
}

/// Read the current working set, not the process-lifetime high-water mark.
#[cfg(target_os = "windows")]
fn resident_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size,
        ..Default::default()
    };
    // SAFETY: GetCurrentProcess returns this process's pseudo-handle. The writable
    // pointer spans one initialized, uniquely borrowed counters struct; cb is its
    // exact byte size. Neither call retains the pointer or owns the pseudo-handle.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } == 0 {
        return None;
    }
    u64::try_from(counters.WorkingSetSize).ok()
}

/// Unsupported diagnostic hosts produce absence rather than a fabricated zero.
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn resident_bytes() -> Option<u64> {
    None
}

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
    /// Whole-process resident bytes at entry; includes Node and its workers.
    pub start_rss_bytes: Option<u64>,
    /// Whole-process resident bytes at exit, including error/unwind completion.
    pub finish_rss_bytes: Option<u64>,
    /// Largest successful endpoint or periodic observation within this interval.
    pub sampled_peak_rss_bytes: Option<u64>,
    /// Successful resident-memory observations, including both endpoints.
    pub rss_samples: u64,
}

/// Opt-in bounded timing history, readable during work and retained after completion.
pub struct ArchivePhaseTrace {
    started: Instant,
    intervals: Mutex<Vec<ArchivePhaseInterval>>,
    truncated: AtomicBool,
    /// Whether this job actually created its optional sampler thread.
    pub rss_sampler_started: AtomicBool,
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
            rss_sampler_started: AtomicBool::new(false),
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
        let rss = resident_bytes();
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
            start_rss_bytes: rss,
            finish_rss_bytes: None,
            sampled_peak_rss_bytes: rss,
            rss_samples: u64::from(rss.is_some()),
        });
        Some(index)
    }

    fn finish(&self, index: usize) {
        let rss = resident_bytes();
        let mut intervals = self
            .intervals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let interval = &mut intervals[index];
        interval.finished_micros = Some(self.elapsed_micros());
        interval.finish_rss_bytes = rss;
        Self::record_rss(interval, rss);
    }

    fn record_rss(interval: &mut ArchivePhaseInterval, rss: Option<u64>) {
        if let Some(bytes) = rss {
            interval.sampled_peak_rss_bytes =
                Some(interval.sampled_peak_rss_bytes.unwrap_or(0).max(bytes));
            interval.rss_samples = interval.rss_samples.saturating_add(1);
        }
    }

    fn sample_rss(&self) {
        let sampled_at = self.elapsed_micros();
        let rss = resident_bytes();
        let mut intervals = self
            .intervals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for interval in intervals.iter_mut() {
            // A closed stage receives its own exit reading, never a late sample.
            // Strict ordering also avoids ambiguous equal-microsecond boundaries.
            if interval.started_micros < sampled_at && interval.finished_micros.is_none() {
                Self::record_rss(interval, rss);
            }
        }
    }
}

/// One opt-in observer per native job, joined even when the worker unwinds.
struct RssSampler {
    stop: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

impl RssSampler {
    fn start(trace: Arc<ArchivePhaseTrace>) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_trace = Arc::clone(&trace);
        let worker = thread::Builder::new()
            .name("slither-archive-rss".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    worker_trace.sample_rss();
                    thread::park_timeout(Duration::from_micros(ARCHIVE_RSS_SAMPLE_INTERVAL_MICROS));
                }
            })
            .ok()?;
        trace.rss_sampler_started.store(true, Ordering::Release);
        Some(Self { stop, worker })
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Release);
        self.worker.thread().unpark();
        let _ = self.worker.join();
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
pub struct ProgressScope(Option<RssSampler>);

impl ProgressScope {
    pub fn enter(counter: Arc<AtomicU64>) -> Self {
        Self::enter_with_trace(counter, None)
    }

    pub fn enter_with_trace(
        counter: Arc<AtomicU64>,
        trace: Option<Arc<ArchivePhaseTrace>>,
    ) -> Self {
        let sampler_trace = trace.clone();
        CURRENT.with(|current| {
            let mut context = current.borrow_mut();
            assert!(
                context.is_none(),
                "nested archive progress jobs are unsupported"
            );
            *context = Some(ProgressContext { counter, trace });
        });
        let sampler = sampler_trace.and_then(RssSampler::start);
        Self(sampler)
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
        if let Some(sampler) = self.0.take() {
            sampler.stop();
        }
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
        assert!(trace.rss_sampler_started.load(Ordering::Acquire));
        // No observer retains the trace once its job guard completes.
        assert_eq!(Arc::strong_count(&trace), 1);
        for interval in intervals {
            let start = interval.start_rss_bytes.unwrap();
            let finish = interval.finish_rss_bytes.unwrap();
            assert!(start > 0 && finish > 0);
            assert!(interval.rss_samples >= 2);
            assert!(interval.sampled_peak_rss_bytes.unwrap() >= start.max(finish));
        }
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
        assert_eq!(Arc::strong_count(&trace), 1);
        let _inactive = ArchivePhaseScope::enter(ArchivePhase::Import);
        advance(100);
        assert_eq!(trace.snapshot().2.len(), 1);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        let next = Arc::new(AtomicU64::new(0));
        let _job = ProgressScope::enter(Arc::clone(&next));
        advance(7);
        assert_eq!(next.load(Ordering::Relaxed), 7);
    }

    /// Invalid nesting preserves the original byte context and cannot leak a sampler.
    #[test]
    fn rejected_nested_job_preserves_the_owner_and_stops_all_observers() {
        let original = Arc::new(AtomicU64::new(0));
        let trace = Arc::new(ArchivePhaseTrace::new());
        let rejected = Arc::new(ArchivePhaseTrace::new());
        {
            let _job =
                ProgressScope::enter_with_trace(Arc::clone(&original), Some(Arc::clone(&trace)));
            let result = std::panic::catch_unwind(|| {
                let _nested = ProgressScope::enter_with_trace(
                    Arc::new(AtomicU64::new(0)),
                    Some(Arc::clone(&rejected)),
                );
            });
            assert!(result.is_err());
            assert!(!rejected.rss_sampler_started.load(Ordering::Acquire));
            assert_eq!(Arc::strong_count(&rejected), 1);
            advance(9);
            assert_eq!(original.load(Ordering::Relaxed), 9);
        }
        assert_eq!(Arc::strong_count(&trace), 1);
    }

    /// Late samples cannot inflate a closed interval's peak or observation count.
    #[test]
    fn finished_phases_do_not_receive_later_process_samples() {
        let trace = ArchivePhaseTrace::new();
        let phase = trace.begin(ArchivePhase::Validation).unwrap();
        trace.sample_rss();
        trace.finish(phase);
        let before = trace.snapshot().2[phase].clone();
        trace.sample_rss();
        let after = trace.snapshot().2[phase].clone();
        assert_eq!(after.rss_samples, before.rss_samples);
        assert_eq!(after.sampled_peak_rss_bytes, before.sampled_peak_rss_bytes);
        assert_eq!(after.finish_rss_bytes, before.finish_rss_bytes);
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
