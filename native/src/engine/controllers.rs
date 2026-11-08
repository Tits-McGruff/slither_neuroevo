//! Wall-time external-controller ownership and staged source selection.
//!
//! Browser players and Protocol 2 RL clients produce actions differently, but
//! once Node has accepted an action both use the same Rust-owned lease. This
//! module deliberately does not evaluate a neural graph. It prepares a checked
//! decision that a later complete control transaction can combine with
//! baseline and neural proposals before any authoritative write.

use super::state::{
    ControllerLease, ControllerLeaseStatus, LatestControllerAction, NormalizedEngineConfig,
    SnakeKind, SnakeState,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Validated wall-time durations governing an external lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerTiming {
    input_hold_ms: u64,
    disconnect_grace_ms: u64,
}

impl ControllerTiming {
    /// Owner-selected initial 500 ms hold and 30-second exclusive grace.
    #[must_use]
    pub const fn approved_defaults() -> Self {
        Self {
            input_hold_ms: 500,
            disconnect_grace_ms: 30_000,
        }
    }

    /// Validate explicit hold and grace durations.
    pub fn new(input_hold_ms: u64, disconnect_grace_ms: u64) -> Result<Self, ControllerError> {
        if input_hold_ms == 0 || disconnect_grace_ms == 0 {
            return Err(ControllerError::InvalidTiming(
                "input hold and disconnect grace must be positive",
            ));
        }
        if input_hold_ms >= disconnect_grace_ms {
            return Err(ControllerError::InvalidTiming(
                "input hold must be shorter than disconnect grace",
            ));
        }
        Ok(Self {
            input_hold_ms,
            disconnect_grace_ms,
        })
    }

    /// Read timing from the already-admitted normalized configuration.
    pub fn from_config(config: &NormalizedEngineConfig) -> Result<Self, ControllerError> {
        Self::new(
            config.controller_input_hold_ms,
            config.controller_disconnect_grace_ms,
        )
    }

    /// Wall milliseconds for which one latest accepted action remains active.
    #[must_use]
    pub const fn input_hold_ms(self) -> u64 {
        self.input_hold_ms
    }

    /// Wall milliseconds for which disconnected ownership remains exclusive.
    #[must_use]
    pub const fn disconnect_grace_ms(self) -> u64 {
        self.disconnect_grace_ms
    }
}

/// Source selected for one externally owned snake at a fixed-step boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExternalControlSource {
    /// The latest action is still within its wall-time hold window.
    HeldAction { turn: f32, boost: bool },
    /// Ownership remains exclusive, but the latest action is stale.
    ReservedNeutral,
    /// Grace has expired and the one explicit neural transition is eligible.
    NeuralTakeover,
}

/// Immutable snapshot used to reject a stale boundary proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LeaseSnapshot {
    lease_id: u64,
    snake_id: u64,
    status: ControllerLeaseStatus,
    connection_id: Option<u64>,
    action_turn_bits: u32,
    action_boost: bool,
    action_client_tick: u64,
    action_is_explicit: bool,
    action_arrival_sequence: u64,
    action_accepted_at_ms: u64,
    last_observed_at_ms: u64,
    disconnected_at_ms: Option<u64>,
    input_hold_expires_at_ms: Option<u64>,
    grace_expires_at_ms: Option<u64>,
    takeover_committed_at_ms: Option<u64>,
}

impl LeaseSnapshot {
    fn capture(lease: &ControllerLease) -> Self {
        Self {
            lease_id: lease.id,
            snake_id: lease.snake_id,
            status: lease.status,
            connection_id: lease.connection_id,
            action_turn_bits: lease.latest_action.turn.to_bits(),
            action_boost: lease.latest_action.boost,
            action_client_tick: lease.latest_action.client_tick,
            action_is_explicit: lease.latest_action.is_explicit,
            action_arrival_sequence: lease.latest_action.arrival_sequence,
            action_accepted_at_ms: lease.latest_action.accepted_at_ms,
            last_observed_at_ms: lease.last_observed_at_ms,
            disconnected_at_ms: lease.disconnected_at_ms,
            input_hold_expires_at_ms: lease.input_hold_expires_at_ms,
            grace_expires_at_ms: lease.grace_expires_at_ms,
            takeover_committed_at_ms: lease.takeover_committed_at_ms,
        }
    }

    fn matches(self, lease: &ControllerLease) -> bool {
        self == Self::capture(lease)
    }
}

/// Authority-relevant snake state guarded between proposal and commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SnakeControlSnapshot {
    id: u64,
    kind: SnakeKind,
    alive: bool,
    brain: Option<super::state::BrainHandle>,
    turn_bits: u32,
    previous_turn_bits: u32,
    input_boost: bool,
    previous_input_boost: bool,
}

impl SnakeControlSnapshot {
    fn capture(snake: &SnakeState) -> Self {
        Self {
            id: snake.id,
            kind: snake.kind,
            alive: snake.alive,
            brain: snake.brain,
            turn_bits: snake.turn.to_bits(),
            previous_turn_bits: snake.previous_turn.to_bits(),
            input_boost: snake.input_boost,
            previous_input_boost: snake.previous_input_boost,
        }
    }

    fn matches(self, snake: &SnakeState) -> bool {
        self == Self::capture(snake)
    }
}

/// Checked, not-yet-committed lease/source result for one step boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerBoundaryProposal {
    expected: LeaseSnapshot,
    expected_snake: SnakeControlSnapshot,
    source: ExternalControlSource,
    next_status: ControllerLeaseStatus,
    next_takeover_committed_at_ms: Option<u64>,
    boundary_at_ms: u64,
}

impl ControllerBoundaryProposal {
    /// Lease identity guarded by this proposal.
    #[must_use]
    pub const fn lease_id(&self) -> u64 {
        self.expected.lease_id
    }

    /// Snake identity guarded by this proposal.
    #[must_use]
    pub const fn snake_id(&self) -> u64 {
        self.expected.snake_id
    }

    /// Selected exclusive external or neural source.
    #[must_use]
    pub const fn source(&self) -> ExternalControlSource {
        self.source
    }

    /// Whether this boundary performs the one external-to-neural transition.
    #[must_use]
    pub const fn begins_neural_takeover(&self) -> bool {
        !matches!(self.expected.status, ControllerLeaseStatus::NeuralTakeover)
            && matches!(self.next_status, ControllerLeaseStatus::NeuralTakeover)
    }
}

/// Prepare one latest-value action replacement without changing authority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatestActionProposal {
    expected: LeaseSnapshot,
    connection_id: u64,
    action: LatestControllerAction,
    boundary_at_ms: u64,
}

/// One already wire-validated latest-value action delivered by the thin bridge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatestActionInput {
    /// Optional client correlation, independent of diagnostic tick or native queue sequence.
    pub client_action_id: Option<u64>,
    /// Controller assignment epoch named by the action.
    pub lease_id: u64,
    /// Live transport connection identity named by the action.
    pub connection_id: u64,
    /// Finite steering value in `[-1, 1]`.
    pub turn: f32,
    /// Requested boost state.
    pub boost: bool,
    /// Client tick retained only for diagnostics and Protocol 2 compatibility.
    pub client_tick: u64,
    /// Monotonic Node-assigned command arrival sequence.
    pub arrival_sequence: u64,
    /// Monotonic elapsed milliseconds when Node accepted the command.
    pub accepted_at_ms: u64,
}

/// Prepare one disconnect transition without changing authority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisconnectProposal {
    expected: LeaseSnapshot,
    expected_snake: SnakeControlSnapshot,
    disconnected_at_ms: u64,
    boundary_at_ms: u64,
    input_hold_expires_at_ms: u64,
    grace_expires_at_ms: u64,
    next_status: ControllerLeaseStatus,
    source: ExternalControlSource,
}

/// Correlated reconnect identity; token generation and uniqueness belong to the
/// enclosing assignment transaction, never to simulation RNG.
pub struct ReclaimInput<'a> {
    pub kind: super::state::ControllerKind,
    pub scope: &'a str,
    pub resume_token: &'a str,
    /// Fresh, globally unique OS-entropy token checked before reliable delivery.
    pub next_resume_token: String,
    pub connection_id: u64,
    pub arrival_sequence: u64,
    /// Native receipt time, preserved when an earlier source step was pending.
    pub received_at_ms: u64,
    pub boundary_at_ms: u64,
}

/// A reconnect remains invisible until its reliable assignment is admitted and
/// delivered. Dropping this proposal leaves the old token and ownership intact.
#[derive(Clone, Debug, PartialEq)]
pub struct ReclaimProposal {
    expected: ControllerLease,
    expected_snake: SnakeControlSnapshot,
    reward: Option<super::external_reward::ExternalRewardLedger>,
    connection_id: u64,
    arrival_sequence: u64,
    received_at_ms: u64,
    boundary_at_ms: u64,
    next_resume_token: String,
}

impl ReclaimProposal {
    /// Assignment identity retained while its reliable send is unresolved.
    #[must_use]
    pub const fn lease_id(&self) -> u64 {
        self.expected.id
    }

    /// Connection that may own the lease after successful delivery.
    #[must_use]
    pub const fn connection_id(&self) -> u64 {
        self.connection_id
    }

    /// Already-validated token to include in the reliable assignment.
    #[must_use]
    pub fn resume_token(&self) -> &str {
        &self.next_resume_token
    }
}

/// Match only one live reserved legacy identity. Connected, expired, dead, and
/// other-run leases cannot be stolen; ambiguous names never pick by array order.
pub fn select_legacy_reclaim<'a>(
    world: &'a super::state::WorldState,
    kind: super::state::ControllerKind,
    scope: &str,
    identity_key: &str,
    boundary_at_ms: u64,
    timing: ControllerTiming,
) -> Result<Option<&'a ControllerLease>, ControllerError> {
    if identity_key.is_empty() {
        return Ok(None);
    }
    if identity_key.len() > 128 || identity_key.contains('\0') {
        return Err(ControllerError::ReclaimRejected("invalid"));
    }
    let mut matched = None;
    for lease in &world.controller_leases {
        if lease.kind != kind
            || lease.scope != scope
            || lease.identity_key != identity_key
            || !matches!(
                lease.status,
                ControllerLeaseStatus::HoldingLastInput | ControllerLeaseStatus::ReservedNeutral
            )
        {
            continue;
        }
        let (_, _, grace) = validate_disconnected_deadlines(lease, timing)?;
        if boundary_at_ms < lease.last_observed_at_ms {
            return Err(ControllerError::WallClockRegressed {
                previous_ms: lease.last_observed_at_ms,
                current_ms: boundary_at_ms,
            });
        }
        if boundary_at_ms >= grace
            || !world.snakes.iter().any(|snake| {
                snake.id == lease.snake_id && snake.alive && snake.kind == SnakeKind::External
            })
        {
            continue;
        }
        if matched.is_some() {
            return Err(ControllerError::ReclaimRejected("ambiguous"));
        }
        matched = Some(lease);
    }
    Ok(matched)
}

/// Stage token-based reclaim without changing controls, RNG, or recurrent state.
pub fn prepare_reclaim(
    lease: &ControllerLease,
    snake: &SnakeState,
    input: ReclaimInput<'_>,
    timing: ControllerTiming,
) -> Result<ReclaimProposal, ControllerError> {
    validate_pair(lease, snake)?;
    if input.scope != lease.scope
        || input.kind != lease.kind
        || input.resume_token != lease.resume_token
        || input.resume_token.is_empty()
        || input.connection_id == 0
    {
        return Err(ControllerError::ReclaimRejected("invalid"));
    }
    if input.boundary_at_ms < lease.last_observed_at_ms
        || input.boundary_at_ms < input.received_at_ms
    {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.last_observed_at_ms.max(input.received_at_ms),
            current_ms: input.boundary_at_ms,
        });
    }
    if input.received_at_ms < lease.latest_action.accepted_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.latest_action.accepted_at_ms,
            current_ms: input.received_at_ms,
        });
    }
    if lease.status != ControllerLeaseStatus::Connected {
        let (_, _, grace) = validate_disconnected_deadlines(lease, timing)?;
        if lease.status == ControllerLeaseStatus::NeuralTakeover || input.boundary_at_ms >= grace {
            return Err(ControllerError::ReclaimRejected("expired"));
        }
    }
    if !snake.alive {
        return Err(ControllerError::ReclaimRejected("snake-unavailable"));
    }
    if input.arrival_sequence <= lease.latest_action.arrival_sequence {
        return Err(ControllerError::ArrivalSequenceRegressed {
            previous: lease.latest_action.arrival_sequence,
            current: input.arrival_sequence,
        });
    }
    if input.next_resume_token.len() != super::external_replacement::RESUME_TOKEN_LENGTH
        || !input
            .next_resume_token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        || input.next_resume_token == lease.resume_token
    {
        return Err(ControllerError::ReclaimRejected(
            "invalid replacement token",
        ));
    }
    let mut reward = lease.reward;
    if let Some(ledger) = &mut reward {
        ledger
            .retire_unapplied_command()
            .map_err(|error| ControllerError::ReclaimRejected(error.0))?;
    }
    Ok(ReclaimProposal {
        expected: lease.clone(),
        expected_snake: SnakeControlSnapshot::capture(snake),
        reward,
        connection_id: input.connection_id,
        arrival_sequence: input.arrival_sequence,
        received_at_ms: input.received_at_ms,
        boundary_at_ms: input.boundary_at_ms,
        next_resume_token: input.next_resume_token,
    })
}

/// Commit after the enclosing transaction checks token uniqueness and receives
/// the exact successful local-send receipt. A failed send must drop the proposal.
pub fn commit_reclaim(
    lease: &mut ControllerLease,
    snake: &mut SnakeState,
    proposal: ReclaimProposal,
) -> Result<(), ControllerError> {
    if *lease != proposal.expected || !proposal.expected_snake.matches(snake) {
        return Err(ControllerError::StaleProposal {
            lease_id: proposal.expected.id,
        });
    }
    lease.connection_id = Some(proposal.connection_id);
    lease.resume_token = proposal.next_resume_token;
    lease.status = ControllerLeaseStatus::Connected;
    lease.reward = proposal.reward;
    lease.latest_action = LatestControllerAction {
        is_explicit: false,
        turn: 0.0,
        boost: false,
        client_tick: 0,
        arrival_sequence: proposal.arrival_sequence,
        accepted_at_ms: proposal.received_at_ms,
    };
    lease.last_observed_at_ms = proposal.boundary_at_ms;
    lease.disconnected_at_ms = None;
    lease.input_hold_expires_at_ms = None;
    lease.grace_expires_at_ms = None;
    lease.takeover_committed_at_ms = None;
    apply_external_source(snake, ExternalControlSource::ReservedNeutral);
    Ok(())
}

/// Validate and stage a newest accepted action for a live lease.
///
/// `lease_id` is the assignment epoch supplied by the thin bridge. Requiring
/// both it and the live connection identity prevents an old connection from
/// mutating a reclaimed assignment. The client tick is diagnostic only.
pub fn prepare_latest_action(
    lease: &ControllerLease,
    input: LatestActionInput,
) -> Result<LatestActionProposal, ControllerError> {
    prepare_queued_latest_action(lease, input, input.accepted_at_ms)
}

/// Admit an action received while an earlier step was awaiting delivery. Its
/// hold window starts at receipt, never at this later application boundary.
pub fn prepare_queued_latest_action(
    lease: &ControllerLease,
    input: LatestActionInput,
    boundary_at_ms: u64,
) -> Result<LatestActionProposal, ControllerError> {
    validate_lease_identity(lease)?;
    if lease.id != input.lease_id {
        return Err(ControllerError::StaleLease {
            expected: lease.id,
            actual: input.lease_id,
        });
    }
    if input.connection_id == 0 || lease.connection_id != Some(input.connection_id) {
        return Err(ControllerError::StaleConnection {
            expected: lease.connection_id,
            actual: input.connection_id,
        });
    }
    if lease.status != ControllerLeaseStatus::Connected {
        return Err(ControllerError::LeaseNotConnected(lease.status));
    }
    if !input.turn.is_finite() || !(-1.0..=1.0).contains(&input.turn) {
        return Err(ControllerError::InvalidTurn(input.turn));
    }
    if input.accepted_at_ms < lease.latest_action.accepted_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.latest_action.accepted_at_ms,
            current_ms: input.accepted_at_ms,
        });
    }
    if boundary_at_ms < lease.last_observed_at_ms || boundary_at_ms < input.accepted_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.last_observed_at_ms.max(input.accepted_at_ms),
            current_ms: boundary_at_ms,
        });
    }
    if input.arrival_sequence == 0 || input.arrival_sequence <= lease.latest_action.arrival_sequence
    {
        return Err(ControllerError::ArrivalSequenceRegressed {
            previous: lease.latest_action.arrival_sequence,
            current: input.arrival_sequence,
        });
    }
    Ok(LatestActionProposal {
        expected: LeaseSnapshot::capture(lease),
        connection_id: input.connection_id,
        boundary_at_ms,
        action: LatestControllerAction {
            is_explicit: true,
            turn: input.turn,
            boost: input.boost,
            client_tick: input.client_tick,
            arrival_sequence: input.arrival_sequence,
            accepted_at_ms: input.accepted_at_ms,
        },
    })
}

/// Commit one already-validated latest action replacement.
pub fn commit_latest_action(
    lease: &mut ControllerLease,
    proposal: LatestActionProposal,
) -> Result<(), ControllerError> {
    if !proposal.expected.matches(lease) {
        return Err(ControllerError::StaleProposal {
            lease_id: proposal.expected.lease_id,
        });
    }
    if lease.connection_id != Some(proposal.connection_id)
        || lease.status != ControllerLeaseStatus::Connected
    {
        return Err(ControllerError::StaleProposal { lease_id: lease.id });
    }
    lease.latest_action = proposal.action;
    lease.last_observed_at_ms = proposal.boundary_at_ms;
    Ok(())
}

/// Stage a live socket disconnect and its exact hold/grace deadlines.
pub fn prepare_disconnect(
    lease: &ControllerLease,
    snake: &SnakeState,
    connection_id: u64,
    disconnected_at_ms: u64,
    timing: ControllerTiming,
) -> Result<DisconnectProposal, ControllerError> {
    prepare_queued_disconnect(
        lease,
        snake,
        connection_id,
        disconnected_at_ms,
        disconnected_at_ms,
        timing,
    )
}

/// Preserve the actual close time when its command waited behind a retained step.
/// Applying later never extends the hold or exclusive reclaim grace deadlines.
pub fn prepare_queued_disconnect(
    lease: &ControllerLease,
    snake: &SnakeState,
    connection_id: u64,
    disconnected_at_ms: u64,
    boundary_at_ms: u64,
    timing: ControllerTiming,
) -> Result<DisconnectProposal, ControllerError> {
    validate_pair(lease, snake)?;
    if lease.status != ControllerLeaseStatus::Connected {
        return Err(ControllerError::LeaseNotConnected(lease.status));
    }
    if connection_id == 0 || lease.connection_id != Some(connection_id) {
        return Err(ControllerError::StaleConnection {
            expected: lease.connection_id,
            actual: connection_id,
        });
    }
    if disconnected_at_ms < lease.latest_action.accepted_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.latest_action.accepted_at_ms,
            current_ms: disconnected_at_ms,
        });
    }
    if boundary_at_ms < lease.last_observed_at_ms || boundary_at_ms < disconnected_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.last_observed_at_ms.max(disconnected_at_ms),
            current_ms: boundary_at_ms,
        });
    }
    let input_hold_expires_at_ms = checked_deadline(
        lease.latest_action.accepted_at_ms,
        timing.input_hold_ms,
        "controller input hold",
    )?;
    let grace_expires_at_ms = checked_deadline(
        disconnected_at_ms,
        timing.disconnect_grace_ms,
        "controller disconnect grace",
    )?;
    let (next_status, source) = if boundary_at_ms < input_hold_expires_at_ms {
        (
            ControllerLeaseStatus::HoldingLastInput,
            ExternalControlSource::HeldAction {
                turn: lease.latest_action.turn,
                boost: lease.latest_action.boost,
            },
        )
    } else {
        (
            ControllerLeaseStatus::ReservedNeutral,
            ExternalControlSource::ReservedNeutral,
        )
    };
    Ok(DisconnectProposal {
        expected: LeaseSnapshot::capture(lease),
        expected_snake: SnakeControlSnapshot::capture(snake),
        disconnected_at_ms,
        boundary_at_ms,
        input_hold_expires_at_ms,
        grace_expires_at_ms,
        next_status,
        source,
    })
}

/// Commit one already-validated disconnect and synchronize held snake input.
pub fn commit_disconnect(
    lease: &mut ControllerLease,
    snake: &mut SnakeState,
    proposal: DisconnectProposal,
) -> Result<(), ControllerError> {
    validate_disconnect_proposal(lease, snake, proposal)?;
    commit_disconnect_prevalidated(lease, snake, proposal);
    Ok(())
}

/// Validate one retained disconnect proposal without changing its lease or snake.
///
/// The complete-step coordinator uses this before exposing an external
/// observation to Node. Once validated under the same exclusive staged-world
/// borrow, [`commit_disconnect_prevalidated`] is an infallible choice between
/// the already-checked delivered and failed-send outcomes.
pub(super) fn validate_disconnect_proposal(
    lease: &ControllerLease,
    snake: &SnakeState,
    proposal: DisconnectProposal,
) -> Result<(), ControllerError> {
    validate_pair(lease, snake)?;
    if !proposal.expected.matches(lease) || !proposal.expected_snake.matches(snake) {
        return Err(ControllerError::StaleProposal {
            lease_id: proposal.expected.lease_id,
        });
    }
    Ok(())
}

/// Apply one disconnect whose exact lease and snake snapshots were prevalidated.
///
/// This function deliberately returns no `Result`: it is used only while the
/// coordinator still owns the unchanged staged world that was checked before
/// an external event became visible.
pub(super) fn commit_disconnect_prevalidated(
    lease: &mut ControllerLease,
    snake: &mut SnakeState,
    proposal: DisconnectProposal,
) {
    debug_assert!(validate_disconnect_proposal(lease, snake, proposal).is_ok());
    lease.connection_id = None;
    lease.status = proposal.next_status;
    lease.disconnected_at_ms = Some(proposal.disconnected_at_ms);
    lease.input_hold_expires_at_ms = Some(proposal.input_hold_expires_at_ms);
    lease.grace_expires_at_ms = Some(proposal.grace_expires_at_ms);
    lease.takeover_committed_at_ms = None;
    lease.last_observed_at_ms = proposal.boundary_at_ms;
    apply_external_source(snake, proposal.source);
}

/// Prepare the exclusive source decision for one fixed-step boundary.
pub fn prepare_controller_boundary(
    lease: &ControllerLease,
    snake: &SnakeState,
    now_ms: u64,
    timing: ControllerTiming,
) -> Result<ControllerBoundaryProposal, ControllerError> {
    validate_pair(lease, snake)?;
    if !snake.alive {
        return Err(ControllerError::SnakeUnavailable(snake.id));
    }
    if now_ms < lease.last_observed_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.last_observed_at_ms,
            current_ms: now_ms,
        });
    }
    if now_ms < lease.latest_action.accepted_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.latest_action.accepted_at_ms,
            current_ms: now_ms,
        });
    }
    let expected = LeaseSnapshot::capture(lease);
    let expected_snake = SnakeControlSnapshot::capture(snake);
    let (source, next_status, next_takeover_committed_at_ms) = match lease.status {
        ControllerLeaseStatus::Connected => {
            if lease.connection_id.is_none()
                || lease.disconnected_at_ms.is_some()
                || lease.input_hold_expires_at_ms.is_some()
                || lease.grace_expires_at_ms.is_some()
                || lease.takeover_committed_at_ms.is_some()
            {
                return Err(ControllerError::InvalidLeaseState(
                    "connected lease has disconnect or takeover state",
                ));
            }
            let deadline = checked_deadline(
                lease.latest_action.accepted_at_ms,
                timing.input_hold_ms,
                "connected action hold",
            )?;
            let source = if now_ms < deadline {
                ExternalControlSource::HeldAction {
                    turn: lease.latest_action.turn,
                    boost: lease.latest_action.boost,
                }
            } else {
                ExternalControlSource::ReservedNeutral
            };
            (source, ControllerLeaseStatus::Connected, None)
        }
        ControllerLeaseStatus::HoldingLastInput | ControllerLeaseStatus::ReservedNeutral => {
            let (disconnected, hold_deadline, grace_deadline) =
                validate_disconnected_deadlines(lease, timing)?;
            if now_ms < disconnected {
                return Err(ControllerError::WallClockRegressed {
                    previous_ms: disconnected,
                    current_ms: now_ms,
                });
            }
            if now_ms >= grace_deadline {
                if snake.brain.is_none() {
                    return Err(ControllerError::TakeoverBrainMissing(snake.id));
                }
                (
                    ExternalControlSource::NeuralTakeover,
                    ControllerLeaseStatus::NeuralTakeover,
                    Some(now_ms),
                )
            } else if now_ms < hold_deadline {
                if lease.status != ControllerLeaseStatus::HoldingLastInput {
                    return Err(ControllerError::InvalidLeaseState(
                        "neutral grace cannot precede the input-hold deadline",
                    ));
                }
                (
                    ExternalControlSource::HeldAction {
                        turn: lease.latest_action.turn,
                        boost: lease.latest_action.boost,
                    },
                    ControllerLeaseStatus::HoldingLastInput,
                    None,
                )
            } else {
                (
                    ExternalControlSource::ReservedNeutral,
                    ControllerLeaseStatus::ReservedNeutral,
                    None,
                )
            }
        }
        ControllerLeaseStatus::NeuralTakeover => {
            let (_, _, grace_deadline) = validate_disconnected_deadlines(lease, timing)?;
            let takeover =
                lease
                    .takeover_committed_at_ms
                    .ok_or(ControllerError::InvalidLeaseState(
                        "neural takeover lacks commit wall time",
                    ))?;
            if takeover < grace_deadline {
                return Err(ControllerError::InvalidLeaseState(
                    "neural takeover precedes grace expiry",
                ));
            }
            if now_ms < takeover {
                return Err(ControllerError::WallClockRegressed {
                    previous_ms: takeover,
                    current_ms: now_ms,
                });
            }
            if snake.brain.is_none() {
                return Err(ControllerError::TakeoverBrainMissing(snake.id));
            }
            (
                ExternalControlSource::NeuralTakeover,
                ControllerLeaseStatus::NeuralTakeover,
                Some(takeover),
            )
        }
    };
    Ok(ControllerBoundaryProposal {
        expected,
        expected_snake,
        source,
        next_status,
        next_takeover_committed_at_ms,
        boundary_at_ms: now_ms,
    })
}

/// Commit a prepared lease transition and its external held/neutral input.
///
/// A complete engine step will call this only after neural preparation has
/// also succeeded. For a takeover proposal this commit clears expired external
/// input; the neural output is then the sole control written by that same
/// complete control commit.
pub fn commit_controller_boundary(
    lease: &mut ControllerLease,
    snake: &mut SnakeState,
    proposal: ControllerBoundaryProposal,
) -> Result<(), ControllerError> {
    validate_controller_boundary(lease, snake, proposal)?;
    commit_controller_boundary_prevalidated(lease, snake, proposal);
    Ok(())
}

/// Revalidate one staged boundary before a larger transaction writes anything.
pub(crate) fn validate_controller_boundary(
    lease: &ControllerLease,
    snake: &SnakeState,
    proposal: ControllerBoundaryProposal,
) -> Result<(), ControllerError> {
    validate_pair(lease, snake)?;
    if !proposal.expected.matches(lease) || !proposal.expected_snake.matches(snake) {
        return Err(ControllerError::StaleProposal {
            lease_id: proposal.expected.lease_id,
        });
    }
    Ok(())
}

/// Publish a boundary already checked against the same exclusive lease/snake pair.
pub(crate) fn commit_controller_boundary_prevalidated(
    lease: &mut ControllerLease,
    snake: &mut SnakeState,
    proposal: ControllerBoundaryProposal,
) {
    debug_assert!(validate_controller_boundary(lease, snake, proposal).is_ok());
    lease.status = proposal.next_status;
    lease.takeover_committed_at_ms = proposal.next_takeover_committed_at_ms;
    lease.last_observed_at_ms = proposal.boundary_at_ms;
    if !matches!(proposal.source, ExternalControlSource::NeuralTakeover)
        || proposal.begins_neural_takeover()
    {
        apply_external_source(snake, proposal.source);
    }
}

fn apply_external_source(snake: &mut SnakeState, source: ExternalControlSource) {
    snake.previous_turn = snake.turn;
    snake.previous_input_boost = snake.input_boost;
    match source {
        ExternalControlSource::HeldAction { turn, boost } => {
            snake.turn = turn;
            snake.input_boost = boost;
        }
        ExternalControlSource::ReservedNeutral | ExternalControlSource::NeuralTakeover => {
            snake.turn = 0.0;
            snake.input_boost = false;
        }
    }
}

fn validate_pair(lease: &ControllerLease, snake: &SnakeState) -> Result<(), ControllerError> {
    validate_lease_identity(lease)?;
    if lease.snake_id != snake.id {
        return Err(ControllerError::SnakeMismatch {
            lease_snake_id: lease.snake_id,
            actual_snake_id: snake.id,
        });
    }
    if snake.kind != SnakeKind::External {
        return Err(ControllerError::SnakeNotExternal(snake.id));
    }
    Ok(())
}

fn validate_lease_identity(lease: &ControllerLease) -> Result<(), ControllerError> {
    if lease.id == 0 || lease.id == u64::MAX || lease.snake_id == 0 {
        return Err(ControllerError::InvalidLeaseState(
            "lease and snake identities must be positive",
        ));
    }
    if !lease.latest_action.turn.is_finite() || !(-1.0..=1.0).contains(&lease.latest_action.turn) {
        return Err(ControllerError::InvalidTurn(lease.latest_action.turn));
    }
    if lease.latest_action.arrival_sequence == 0 {
        return Err(ControllerError::InvalidLeaseState(
            "latest action arrival sequence must be positive",
        ));
    }
    if lease.last_observed_at_ms < lease.latest_action.accepted_at_ms {
        return Err(ControllerError::InvalidLeaseState(
            "last observed wall time precedes the latest action",
        ));
    }
    Ok(())
}

fn validate_disconnected_deadlines(
    lease: &ControllerLease,
    timing: ControllerTiming,
) -> Result<(u64, u64, u64), ControllerError> {
    if lease.connection_id.is_some() {
        return Err(ControllerError::InvalidLeaseState(
            "disconnected or taken-over lease retains a live connection",
        ));
    }
    let disconnected = lease
        .disconnected_at_ms
        .ok_or(ControllerError::InvalidLeaseState(
            "disconnected lease lacks disconnect wall time",
        ))?;
    if lease.latest_action.accepted_at_ms > disconnected {
        return Err(ControllerError::InvalidLeaseState(
            "latest action was accepted after disconnect",
        ));
    }
    if lease.last_observed_at_ms < disconnected {
        return Err(ControllerError::InvalidLeaseState(
            "last observed wall time precedes disconnect",
        ));
    }
    let expected_hold = checked_deadline(
        lease.latest_action.accepted_at_ms,
        timing.input_hold_ms,
        "controller input hold",
    )?;
    let expected_grace = checked_deadline(
        disconnected,
        timing.disconnect_grace_ms,
        "controller disconnect grace",
    )?;
    if lease.input_hold_expires_at_ms != Some(expected_hold)
        || lease.grace_expires_at_ms != Some(expected_grace)
    {
        return Err(ControllerError::InvalidLeaseState(
            "stored controller deadlines do not match configured wall durations",
        ));
    }
    if lease.status != ControllerLeaseStatus::NeuralTakeover
        && lease.takeover_committed_at_ms.is_some()
    {
        return Err(ControllerError::InvalidLeaseState(
            "pre-takeover lease contains takeover commit evidence",
        ));
    }
    Ok((disconnected, expected_hold, expected_grace))
}

fn checked_deadline(
    start_ms: u64,
    duration_ms: u64,
    context: &'static str,
) -> Result<u64, ControllerError> {
    start_ms
        .checked_add(duration_ms)
        .ok_or(ControllerError::DeadlineOverflow { context })
}

/// Rejected action, lease transition, or stale staged proposal.
#[derive(Clone, Debug, PartialEq)]
pub enum ControllerError {
    /// Explicit token-reclaim outcome, without creating a fresh assignment.
    ReclaimRejected(&'static str),
    /// Configured wall-time durations are nonsensical.
    InvalidTiming(&'static str),
    /// A supplied turn is non-finite or outside `[-1, 1]`.
    InvalidTurn(f32),
    /// An action named an obsolete lease/assignment epoch.
    StaleLease { expected: u64, actual: u64 },
    /// An action or disconnect named an obsolete socket.
    StaleConnection { expected: Option<u64>, actual: u64 },
    /// Only a connected lease can accept this operation.
    LeaseNotConnected(ControllerLeaseStatus),
    /// Monotonic wall time moved backwards.
    WallClockRegressed { previous_ms: u64, current_ms: u64 },
    /// Checked wall-time deadline addition overflowed.
    DeadlineOverflow { context: &'static str },
    /// A staged proposal no longer matches current lease state.
    StaleProposal { lease_id: u64 },
    /// Lease and supplied snake identities disagree.
    SnakeMismatch {
        lease_snake_id: u64,
        actual_snake_id: u64,
    },
    /// Node command arrival order moved backwards or was duplicated.
    ArrivalSequenceRegressed { previous: u64, current: u64 },
    /// Controller leases may target only external snakes.
    SnakeNotExternal(u64),
    /// A dead/missing snake cannot receive a control decision.
    SnakeUnavailable(u64),
    /// Neural takeover requires an explicitly owned brain.
    TakeoverBrainMissing(u64),
    /// Stored lease fields contradict their status.
    InvalidLeaseState(&'static str),
}

impl Display for ControllerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReclaimRejected(reason) => {
                write!(formatter, "controller reclaim rejected: {reason}")
            }
            Self::InvalidTiming(reason) => write!(formatter, "invalid controller timing: {reason}"),
            Self::InvalidTurn(turn) => write!(formatter, "invalid controller turn {turn}"),
            Self::StaleLease { expected, actual } => write!(
                formatter,
                "stale controller lease {actual}; current assignment is {expected}"
            ),
            Self::StaleConnection { expected, actual } => write!(
                formatter,
                "stale controller connection {actual}; current connection is {expected:?}"
            ),
            Self::LeaseNotConnected(status) => {
                write!(formatter, "controller lease is not connected: {status:?}")
            }
            Self::WallClockRegressed {
                previous_ms,
                current_ms,
            } => write!(
                formatter,
                "controller wall clock regressed from {previous_ms} ms to {current_ms} ms"
            ),
            Self::ArrivalSequenceRegressed { previous, current } => write!(
                formatter,
                "controller arrival sequence regressed from {previous} to {current}"
            ),
            Self::DeadlineOverflow { context } => {
                write!(
                    formatter,
                    "controller deadline overflow while calculating {context}"
                )
            }
            Self::StaleProposal { lease_id } => {
                write!(formatter, "stale controller proposal for lease {lease_id}")
            }
            Self::SnakeMismatch {
                lease_snake_id,
                actual_snake_id,
            } => write!(
                formatter,
                "controller lease targets snake {lease_snake_id}, not {actual_snake_id}"
            ),
            Self::SnakeNotExternal(id) => {
                write!(formatter, "controller lease snake {id} is not external")
            }
            Self::SnakeUnavailable(id) => {
                write!(formatter, "controller lease snake {id} is unavailable")
            }
            Self::TakeoverBrainMissing(id) => {
                write!(
                    formatter,
                    "controller snake {id} has no brain for neural takeover"
                )
            }
            Self::InvalidLeaseState(reason) => {
                write!(formatter, "invalid controller lease state: {reason}")
            }
        }
    }
}

impl Error for ControllerError {}

#[cfg(test)]
mod tests;
