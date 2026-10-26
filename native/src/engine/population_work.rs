//! Shared admission for one coarse population I/O job beside a held authority.
use super::run_start::PendingRunStartTransition;
use std::sync::{Arc, Mutex};

/// Immutable budget captured by the sole coordinator at an untouched boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PopulationWorkAdmission {
    pub remaining_bytes: usize,
    pub running_ceiling_bytes: usize,
    pub source_world_epoch: u64,
    /// Arithmetic identity captured with the held source, for Reset/New Run.
    pub math_backend: Option<super::inference::InferenceMathBackend>,
}

#[derive(Debug, Default)]
struct PopulationWorkState {
    candidate: Option<PendingRunStartTransition>,
    admission: Option<PopulationWorkAdmission>,
    work_serial: u64,
    work_active: bool,
    publication_active: bool,
}

/// Shared only by the native preparation task and the ordered authority commands.
#[derive(Clone, Debug, Default)]
pub struct PreparedImportSlot(Arc<Mutex<PopulationWorkState>>);

impl PreparedImportSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_some(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .candidate
            .is_some()
    }

    /// Reserve only the headroom remaining beside the stable held source.
    pub(crate) fn reserve_admission(
        &self,
        full: usize,
        source: usize,
        epoch: u64,
    ) -> Result<(), String> {
        self.reserve_admission_with_math(full, source, epoch, None)
    }

    /// Reserve the source budget and immutable arithmetic identity at one boundary.
    pub(crate) fn reserve_admission_with_math(
        &self,
        full: usize,
        source: usize,
        epoch: u64,
        math_backend: Option<super::inference::InferenceMathBackend>,
    ) -> Result<(), String> {
        let remaining = full
            .checked_sub(source)
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                format!("population work has no headroom: source={source}, ceiling={full}")
            })?;
        if epoch == 0 {
            return Err("population work requires an authoritative world epoch".into());
        }
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.admission.is_some() || state.work_active || state.publication_active {
            return Err("population work admission is already retained".into());
        }
        if let Some(candidate) = &mut state.candidate {
            if candidate.selected_running_memory_ceiling_bytes() > full {
                return Err("prepared population exceeds its source process ceiling".into());
            }
            candidate
                .restrict_overlap_memory_ceiling(remaining)
                .map_err(|error| error.to_string())?;
        }
        state.admission = Some(PopulationWorkAdmission {
            remaining_bytes: remaining,
            running_ceiling_bytes: full,
            source_world_epoch: epoch,
            math_backend,
        });
        Ok(())
    }

    /// The guard spans compute, result publication and task finalization across threads.
    pub(crate) fn acquire_work(
        &self,
        selected_ceiling: usize,
    ) -> Result<PopulationWorkLease, String> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let admission = state
            .admission
            .ok_or("stage population work before background allocation")?;
        if admission.running_ceiling_bytes != selected_ceiling {
            return Err("population work ceiling differs from the held authority".into());
        }
        if state.work_active || state.publication_active || state.candidate.is_some() {
            return Err("population work or its prepared candidate is already owned".into());
        }
        state.work_serial = state
            .work_serial
            .checked_add(1)
            .ok_or("population work serial exhausted")?;
        state.work_active = true;
        Ok(PopulationWorkLease {
            slot: self.clone(),
            serial: state.work_serial,
            admission,
            release_admission_on_drop: false,
        })
    }

    pub(crate) fn has_admission(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .admission
            .is_some()
    }

    pub(crate) fn acquire_unstarted_work(
        &self,
        full: usize,
        source: usize,
        epoch: u64,
    ) -> Result<PopulationWorkLease, String> {
        self.reserve_admission(full, source, epoch)?;
        match self.acquire_work(full) {
            Ok(mut lease) => {
                lease.release_admission_on_drop = true;
                Ok(lease)
            }
            Err(error) => {
                self.cancel()?;
                Err(error)
            }
        }
    }

    pub fn put(
        &self,
        mut candidate: PendingRunStartTransition,
    ) -> Result<(), Box<PendingRunStartTransition>> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.candidate.is_some() {
            return Err(Box::new(candidate));
        }
        if let Some(admission) = state.admission {
            if candidate
                .restrict_overlap_memory_ceiling(admission.remaining_bytes)
                .is_err()
                || candidate.selected_running_memory_ceiling_bytes()
                    > admission.running_ceiling_bytes
            {
                return Err(Box::new(candidate));
            }
        }
        state.publication_active = false;
        state.candidate = Some(candidate);
        Ok(())
    }

    /// Remove a discarded candidate without releasing its source admission.
    pub fn take(&self) -> Option<PendingRunStartTransition> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.work_active || state.publication_active {
            return None;
        }
        state.candidate.take()
    }

    /// Keep the slot unavailable until the old owner has actually been retired.
    pub(crate) fn take_for_publication(
        &self,
        epoch: u64,
    ) -> Result<PendingRunStartTransition, String> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let admission = state
            .admission
            .ok_or("population work lacks source admission")?;
        if admission.source_world_epoch != epoch || state.work_active || state.publication_active {
            return Err("population publication is busy or belongs to another source".into());
        }
        let candidate = state
            .candidate
            .take()
            .ok_or("prepared population is missing")?;
        state.publication_active = true;
        Ok(candidate)
    }

    /// Retain a failed activation without readmitting or mutating its already sealed owner.
    pub(crate) fn retain_failed_publication(
        &self,
        candidate: PendingRunStartTransition,
    ) -> Result<(), Box<PendingRunStartTransition>> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if !state.publication_active || state.work_active || state.candidate.is_some() {
            return Err(Box::new(candidate));
        }
        state.candidate = Some(candidate);
        state.publication_active = false;
        Ok(())
    }

    pub(crate) fn complete_publication(&self) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        debug_assert!(state.publication_active && !state.work_active && state.candidate.is_none());
        state.publication_active = false;
        state.admission = None;
    }

    /// Cancellation cannot resume physics while a worker still owns allocations.
    pub(crate) fn cancel(&self) -> Result<(), String> {
        let candidate = {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.work_active || state.publication_active {
                return Err("population work is still running".into());
            }
            state.admission = None;
            state.candidate.take()
        };
        drop(candidate);
        Ok(())
    }
}

impl PartialEq for PreparedImportSlot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for PreparedImportSlot {}

/// A single transferable lease. Started owners keep their hold until ordered cancellation;
/// an unstarted read-only owner releases its temporary admission on drop.
#[derive(Debug)]
pub(crate) struct PopulationWorkLease {
    slot: PreparedImportSlot,
    serial: u64,
    pub admission: PopulationWorkAdmission,
    release_admission_on_drop: bool,
}
impl Drop for PopulationWorkLease {
    fn drop(&mut self) {
        let mut state = self.slot.0.lock().unwrap_or_else(|p| p.into_inner());
        debug_assert_eq!(state.work_serial, self.serial);
        state.work_active = false;
        if self.release_admission_on_drop {
            debug_assert!(state.candidate.is_none() && !state.publication_active);
            state.admission = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_keeps_source_held_until_all_background_work_is_released() {
        let slot = PreparedImportSlot::new();
        assert!(slot.acquire_work(100).is_err());
        slot.reserve_admission_with_math(
            100,
            60,
            1,
            Some(super::super::inference::InferenceMathBackend::Sse2),
        )
        .unwrap();
        let lease = slot.acquire_work(100).unwrap();
        assert_eq!(
            lease.admission.math_backend,
            Some(super::super::inference::InferenceMathBackend::Sse2)
        );
        assert_eq!(lease.admission.remaining_bytes, 40);
        assert!(slot.acquire_work(100).is_err());
        assert!(slot.cancel().is_err());
        assert!(slot.reserve_admission(100, 60, 1).is_err());
        drop(lease);
        slot.cancel().unwrap();
        assert!(slot.acquire_work(100).is_err());
        slot.reserve_admission(100, 25, 2).unwrap();
        let lease = slot.acquire_work(100).unwrap();
        assert_eq!(lease.admission.remaining_bytes, 75);
    }
    #[test]
    fn worker_lease_can_cross_threads_without_releasing_source_admission() {
        let slot = PreparedImportSlot::new();
        slot.reserve_admission(100, 50, 1).unwrap();
        let lease = slot.acquire_work(100).unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
            drop(lease)
        });
        assert!(slot.cancel().is_err());
        release.send(()).unwrap();
        worker.join().unwrap();
        assert!(slot.has_admission());
        slot.cancel().unwrap();
    }

    #[test]
    fn publication_blocks_new_jobs_until_success_or_candidate_rollback() {
        let full = 4 * 1024 * 1024 * 1024;
        let slot = PreparedImportSlot::new();
        slot.reserve_admission(full, full / 2, 1).unwrap();
        let lease = slot.acquire_work(full).unwrap();
        let candidate = super::super::fresh_run::prepare_stage6a_p0_fresh_run(
            super::super::fresh_run::Stage6aP0FreshRunRequest {
                run_id: "publication-lease".into(),
                seed: 42,
                memory_ceiling_bytes: full,
            },
        )
        .unwrap();
        slot.put(candidate).unwrap();
        assert!(slot.take_for_publication(1).is_err());
        assert!(slot.is_some());
        drop(lease);
        assert!(slot.take_for_publication(2).is_err());
        let candidate = slot.take_for_publication(1).unwrap();
        assert!(slot.acquire_work(full).is_err());
        assert!(slot.cancel().is_err());
        slot.put(candidate).unwrap();
        let candidate = slot.take_for_publication(1).unwrap();
        slot.complete_publication();
        assert!(!slot.has_admission());
        assert!(!slot.is_some());
        drop(candidate);
    }

    #[test]
    fn unstarted_readonly_lease_releases_its_temporary_admission() {
        let slot = PreparedImportSlot::new();
        let lease = slot.acquire_unstarted_work(100, 60, 1).unwrap();
        assert!(slot.has_admission());
        assert!(slot.acquire_unstarted_work(100, 60, 1).is_err());
        drop(lease);
        assert!(!slot.has_admission());
        drop(slot.acquire_unstarted_work(100, 60, 1).unwrap());
        assert!(!slot.has_admission());
    }

    #[test]
    fn unfit_and_mismatched_admission_fail_without_owning_work() {
        let slot = PreparedImportSlot::new();
        for (full, source, epoch) in [(0, 0, 1), (100, 100, 1), (100, 101, 1), (100, 50, 0)] {
            assert!(slot.reserve_admission(full, source, epoch).is_err());
        }
        slot.reserve_admission(100, 50, 1).unwrap();
        assert!(slot.acquire_work(101).is_err());
        slot.cancel().unwrap();
    }
}
