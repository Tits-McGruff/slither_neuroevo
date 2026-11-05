//! Copyable command chronology, staged with ordinary authoritative world changes.

use crate::contracts::{
    ExternalRewardAppliedCommandV1, ExternalRewardCommandV1, ExternalRewardControlSourceV1,
    ExternalRewardControlV1,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Command {
    sequence: u64,
    client_action_id: Option<u64>,
    client_tick: u64,
    accepted_at_step: u64,
}

impl Command {
    fn wire(self) -> ExternalRewardCommandV1 {
        ExternalRewardCommandV1 {
            sequence: format!("{:016x}", self.sequence),
            client_action_id: self.client_action_id.map(|value| format!("{value:016x}")),
            client_tick: format!("{:016x}", self.client_tick),
            accepted_at_step: format!("{:016x}", self.accepted_at_step),
        }
    }

    fn hash_into(self, hash: &mut Sha256) {
        for value in [
            self.sequence,
            self.client_action_id.unwrap_or(0),
            self.client_tick,
            self.accepted_at_step,
        ] {
            hash.update(value.to_le_bytes());
        }
    }
}

/// No event history or per-step allocation. Receipt and selection remain different facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExternalCommandLedger {
    accepted: Option<Command>,
    applied: Option<(Command, u64)>,
    source: Option<ExternalRewardControlSourceV1>,
    selected_at_step: Option<u64>,
    accepted_count: u64,
    applied_count: u64,
    superseded_count: u64,
}

fn increment(value: u64) -> Result<u64, &'static str> {
    value
        .checked_add(1)
        .filter(|next| *next <= 9_007_199_254_740_991)
        .ok_or("external command counter overflow")
}

impl ExternalCommandLedger {
    /// Record only after native identity/ordering validation, before committing the lease.
    pub fn received(
        &mut self,
        sequence: u64,
        client_action_id: Option<u64>,
        client_tick: u64,
        step: u64,
    ) -> Result<(), &'static str> {
        if sequence == 0
            || client_action_id == Some(0)
            || self
                .accepted
                .is_some_and(|old| sequence <= old.sequence || step < old.accepted_at_step)
            || self
                .applied
                .is_some_and(|(_, applied_step)| step < applied_step)
        {
            return Err("invalid external command receipt chronology");
        }
        let mut next = *self;
        if self
            .accepted
            .is_some_and(|old| self.applied.map(|value| value.0.sequence) != Some(old.sequence))
        {
            next.superseded_count = increment(next.superseded_count)?;
        }
        next.accepted_count = increment(next.accepted_count)?;
        next.accepted = Some(Command {
            sequence,
            client_action_id,
            client_tick,
            accepted_at_step: step,
        });
        *self = next;
        Ok(())
    }

    /// Select a command for staged physics. A zero sequence is implicit neutral spawn input.
    pub fn selected(
        &mut self,
        sequence: u64,
        source: ExternalRewardControlSourceV1,
        step: u64,
    ) -> Result<(), &'static str> {
        let mut next = *self;
        if self.selected_at_step.is_some_and(|old| step < old)
            || self.accepted.is_some_and(|old| step < old.accepted_at_step)
        {
            return Err("external control selection boundary regressed");
        }
        let source = if source == ExternalRewardControlSourceV1::Held && sequence == 0 {
            ExternalRewardControlSourceV1::Neutral
        } else {
            source
        };
        if source == ExternalRewardControlSourceV1::Held {
            let accepted = self
                .accepted
                .filter(|command| command.sequence == sequence)
                .ok_or("selected external command was never accepted")?;
            if step < accepted.accepted_at_step || self.applied.is_some_and(|(_, old)| step < old) {
                return Err("external command selection boundary regressed");
            }
            if self.applied.map(|value| value.0.sequence) != Some(sequence) {
                next.applied_count = increment(next.applied_count)?;
                next.applied = Some((accepted, step));
            }
        }
        next.source = Some(source);
        next.selected_at_step = Some(step);
        *self = next;
        Ok(())
    }

    /// Pure wire capture, serialized outside the physics kernels.
    pub fn wire(self) -> ExternalRewardControlV1 {
        ExternalRewardControlV1 {
            source: self.source,
            selected_at_step: self.selected_at_step.map(|step| format!("{step:016x}")),
            accepted_command: self.accepted.map(Command::wire),
            applied_command: self
                .applied
                .map(|(command, step)| ExternalRewardAppliedCommandV1 {
                    command: command.wire(),
                    first_applied_step: format!("{step:016x}"),
                }),
            accepted_count: self.accepted_count,
            applied_count: self.applied_count,
            superseded_count: self.superseded_count,
        }
    }

    /// Historical delivery captures cannot claim future receipt or execution counts.
    pub fn no_later_than(self, current: Self) -> bool {
        self.accepted_count <= current.accepted_count
            && self.applied_count <= current.applied_count
            && self.superseded_count <= current.superseded_count
    }

    /// Cover provenance with the existing detached-source mutation guard.
    pub fn hash_into(self, hash: &mut Sha256) {
        hash.update([match self.source {
            None => 0,
            Some(ExternalRewardControlSourceV1::Held) => 1,
            Some(ExternalRewardControlSourceV1::Neutral) => 2,
            Some(ExternalRewardControlSourceV1::Neural) => 3,
        }]);
        for value in [
            self.accepted_count,
            self.applied_count,
            self.superseded_count,
        ] {
            hash.update(value.to_le_bytes());
        }
        hash.update([u8::from(self.selected_at_step.is_some())]);
        hash.update(self.selected_at_step.unwrap_or(0).to_le_bytes());
        hash.update([
            u8::from(self.accepted.is_some()),
            u8::from(self.applied.is_some()),
        ]);
        if let Some(command) = self.accepted {
            command.hash_into(hash);
        }
        if let Some((command, step)) = self.applied {
            command.hash_into(hash);
            hash.update(step.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_supersession_and_physical_selection_are_distinct() {
        let mut ledger = ExternalCommandLedger::default();
        ledger.received(10, Some(1), 3, 5).unwrap();
        assert!(ledger.wire().applied_command.is_none());
        ledger.received(11, Some(2), 4, 5).unwrap();
        let pending = ledger.wire();
        assert_eq!(pending.accepted_count, 2);
        assert_eq!(pending.superseded_count, 1);
        assert_eq!(pending.applied_count, 0);
        ledger
            .selected(11, ExternalRewardControlSourceV1::Held, 5)
            .unwrap();
        ledger
            .selected(11, ExternalRewardControlSourceV1::Held, 6)
            .unwrap();
        let held = ledger.wire();
        assert_eq!(held.applied_count, 1);
        assert_eq!(held.selected_at_step.as_deref(), Some("0000000000000006"));
        assert_eq!(
            held.applied_command.as_ref().unwrap().first_applied_step,
            "0000000000000005"
        );
        assert_eq!(
            held.applied_command
                .unwrap()
                .command
                .client_action_id
                .as_deref(),
            Some("0000000000000002")
        );
        ledger
            .selected(11, ExternalRewardControlSourceV1::Neutral, 7)
            .unwrap();
        assert_eq!(
            ledger.wire().source,
            Some(ExternalRewardControlSourceV1::Neutral)
        );
        assert_eq!(ledger.wire().applied_count, 1);
    }

    #[test]
    fn invalid_receipts_and_selection_leave_the_ledger_unchanged() {
        let mut ledger = ExternalCommandLedger::default();
        ledger.received(10, Some(1), 3, 5).unwrap();
        let before = ledger;
        assert!(ledger.received(10, Some(2), 4, 6).is_err());
        assert!(ledger.received(11, Some(0), 4, 6).is_err());
        assert!(ledger
            .selected(10, ExternalRewardControlSourceV1::Held, 4)
            .is_err());
        assert!(ledger
            .selected(11, ExternalRewardControlSourceV1::Held, 6)
            .is_err());
        assert_eq!(ledger, before);
    }

    #[test]
    fn implicit_spawn_input_is_neutral_and_never_counts_as_a_client_command() {
        let mut ledger = ExternalCommandLedger::default();
        ledger
            .selected(0, ExternalRewardControlSourceV1::Held, 50)
            .unwrap();
        let wire = ledger.wire();
        assert_eq!(wire.source, Some(ExternalRewardControlSourceV1::Neutral));
        assert_eq!(wire.accepted_count, 0);
        assert_eq!(wire.applied_count, 0);
        assert!(wire.applied_command.is_none());
    }
}
