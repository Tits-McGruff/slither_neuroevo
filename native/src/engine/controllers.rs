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
}

/// One already wire-validated latest-value action delivered by the thin bridge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatestActionInput {
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
    input_hold_expires_at_ms: u64,
    grace_expires_at_ms: u64,
    next_status: ControllerLeaseStatus,
    source: ExternalControlSource,
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
    if input.accepted_at_ms < lease.last_observed_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.last_observed_at_ms,
            current_ms: input.accepted_at_ms,
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
        action: LatestControllerAction {
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
    lease.last_observed_at_ms = proposal.action.accepted_at_ms;
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
    if disconnected_at_ms < lease.last_observed_at_ms {
        return Err(ControllerError::WallClockRegressed {
            previous_ms: lease.last_observed_at_ms,
            current_ms: disconnected_at_ms,
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
    let (next_status, source) = if disconnected_at_ms < input_hold_expires_at_ms {
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
    validate_pair(lease, snake)?;
    if !proposal.expected.matches(lease) || !proposal.expected_snake.matches(snake) {
        return Err(ControllerError::StaleProposal {
            lease_id: proposal.expected.lease_id,
        });
    }
    lease.connection_id = None;
    lease.status = proposal.next_status;
    lease.disconnected_at_ms = Some(proposal.disconnected_at_ms);
    lease.input_hold_expires_at_ms = Some(proposal.input_hold_expires_at_ms);
    lease.grace_expires_at_ms = Some(proposal.grace_expires_at_ms);
    lease.takeover_committed_at_ms = None;
    lease.last_observed_at_ms = proposal.disconnected_at_ms;
    apply_external_source(snake, proposal.source);
    Ok(())
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
    validate_pair(lease, snake)?;
    if !proposal.expected.matches(lease) || !proposal.expected_snake.matches(snake) {
        return Err(ControllerError::StaleProposal {
            lease_id: proposal.expected.lease_id,
        });
    }
    lease.status = proposal.next_status;
    lease.takeover_committed_at_ms = proposal.next_takeover_committed_at_ms;
    lease.last_observed_at_ms = proposal.boundary_at_ms;
    apply_external_source(snake, proposal.source);
    Ok(())
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
mod tests {
    use super::*;
    use crate::engine::state::{
        BaselineStrategyState, BodyRange, BrainHandle, ControllerKind, WorldPoint,
    };

    const TIMING: ControllerTiming = ControllerTiming {
        input_hold_ms: 500,
        disconnect_grace_ms: 30_000,
    };

    fn snake() -> SnakeState {
        SnakeState {
            id: 9,
            frame_v1_id: 9,
            kind: SnakeKind::External,
            alive: true,
            population_slot: None,
            brain: Some(BrainHandle { id: 99, epoch: 4 }),
            baseline_slot: None,
            baseline_strategy: None::<BaselineStrategyState>,
            position: WorldPoint { x: 0.0, y: 0.0 },
            previous_position: WorldPoint { x: 0.0, y: 0.0 },
            direction: 0.0,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 5.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange { start: 0, len: 5 },
            skin: 0,
        }
    }

    fn lease() -> ControllerLease {
        ControllerLease {
            id: 7,
            snake_id: 9,
            kind: ControllerKind::Player,
            connection_id: Some(11),
            scope: "run".into(),
            resume_token: "token".into(),
            status: ControllerLeaseStatus::Connected,
            latest_action: LatestControllerAction {
                turn: 0.25,
                boost: true,
                client_tick: 1,
                arrival_sequence: 1,
                accepted_at_ms: 1_000,
            },
            last_observed_at_ms: 1_000,
            disconnected_at_ms: None,
            input_hold_expires_at_ms: None,
            grace_expires_at_ms: None,
            takeover_committed_at_ms: None,
        }
    }

    fn action(
        turn: f32,
        boost: bool,
        client_tick: u64,
        arrival_sequence: u64,
        accepted_at_ms: u64,
    ) -> LatestActionInput {
        LatestActionInput {
            lease_id: 7,
            connection_id: 11,
            turn,
            boost,
            client_tick,
            arrival_sequence,
            accepted_at_ms,
        }
    }

    #[test]
    fn latest_value_replaces_old_action_and_rejects_stale_assignment() {
        let mut lease = lease();
        let first = prepare_latest_action(&lease, action(-0.5, true, 2, 2, 1_100))
            .expect("first latest action should stage");
        commit_latest_action(&mut lease, first).expect("first action should commit");
        let newest = prepare_latest_action(&lease, action(0.75, false, 3, 3, 1_120))
            .expect("newest action should stage");
        commit_latest_action(&mut lease, newest).expect("newest action should replace first");

        let proposal = prepare_controller_boundary(&lease, &snake(), 1_200, TIMING)
            .expect("fresh action should select");
        assert_eq!(
            proposal.source(),
            ExternalControlSource::HeldAction {
                turn: 0.75,
                boost: false
            }
        );
        assert!(matches!(
            prepare_latest_action(
                &lease,
                LatestActionInput {
                    lease_id: 6,
                    ..action(0.0, false, 4, 4, 1_130)
                }
            ),
            Err(ControllerError::StaleLease {
                expected: 7,
                actual: 6
            })
        ));
        assert!(matches!(
            prepare_latest_action(
                &lease,
                LatestActionInput {
                    connection_id: 10,
                    ..action(0.0, false, 4, 4, 1_130)
                }
            ),
            Err(ControllerError::StaleConnection {
                expected: Some(11),
                actual: 10
            })
        ));
    }

    #[test]
    fn connected_action_holds_for_exactly_500_ms_then_becomes_neutral() {
        let lease = lease();
        let fresh = prepare_controller_boundary(&lease, &snake(), 1_499, TIMING)
            .expect("action before deadline should hold");
        assert!(matches!(
            fresh.source(),
            ExternalControlSource::HeldAction { .. }
        ));
        let expired = prepare_controller_boundary(&lease, &snake(), 1_500, TIMING)
            .expect("action at deadline should neutralize");
        assert_eq!(expired.source(), ExternalControlSource::ReservedNeutral);
        assert!(!expired.begins_neural_takeover());
    }

    #[test]
    fn disconnect_holds_then_neutralizes_then_takes_over_once() {
        let mut lease = lease();
        let mut snake = snake();
        let disconnect =
            prepare_disconnect(&lease, &snake, 11, 1_100, TIMING).expect("disconnect should stage");
        commit_disconnect(&mut lease, &mut snake, disconnect).expect("disconnect should commit");
        assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);
        assert_eq!(snake.turn, 0.25);
        assert!(snake.input_boost);

        let held = prepare_controller_boundary(&lease, &snake, 1_499, TIMING)
            .expect("held boundary should stage");
        commit_controller_boundary(&mut lease, &mut snake, held)
            .expect("held boundary should commit");
        assert_eq!(lease.status, ControllerLeaseStatus::HoldingLastInput);

        let neutral = prepare_controller_boundary(&lease, &snake, 1_500, TIMING)
            .expect("neutral boundary should stage");
        commit_controller_boundary(&mut lease, &mut snake, neutral)
            .expect("neutral boundary should commit");
        assert_eq!(lease.status, ControllerLeaseStatus::ReservedNeutral);
        assert_eq!(snake.turn, 0.0);
        assert!(!snake.input_boost);

        let still_reserved = prepare_controller_boundary(&lease, &snake, 31_099, TIMING)
            .expect("pre-expiry grace should remain exclusive");
        assert_eq!(
            still_reserved.source(),
            ExternalControlSource::ReservedNeutral
        );
        assert!(!still_reserved.begins_neural_takeover());

        let takeover = prepare_controller_boundary(&lease, &snake, 31_100, TIMING)
            .expect("grace expiry should stage takeover");
        assert_eq!(takeover.source(), ExternalControlSource::NeuralTakeover);
        assert!(takeover.begins_neural_takeover());
        commit_controller_boundary(&mut lease, &mut snake, takeover)
            .expect("takeover should commit");
        assert_eq!(lease.status, ControllerLeaseStatus::NeuralTakeover);
        assert_eq!(lease.takeover_committed_at_ms, Some(31_100));
        assert_eq!(snake.turn, 0.0);
        assert!(!snake.input_boost);

        let later = prepare_controller_boundary(&lease, &snake, 31_101, TIMING)
            .expect("committed takeover should remain neural");
        assert_eq!(later.source(), ExternalControlSource::NeuralTakeover);
        assert!(!later.begins_neural_takeover());
    }

    #[test]
    fn stale_action_at_disconnect_enters_neutral_grace_immediately() {
        let mut lease = lease();
        let mut snake = snake();
        let disconnect = prepare_disconnect(&lease, &snake, 11, 2_000, TIMING)
            .expect("stale disconnect should stage");
        commit_disconnect(&mut lease, &mut snake, disconnect)
            .expect("stale disconnect should commit");
        assert_eq!(lease.status, ControllerLeaseStatus::ReservedNeutral);
        assert_eq!(lease.input_hold_expires_at_ms, Some(1_500));
        assert_eq!(lease.grace_expires_at_ms, Some(32_000));
        assert_eq!(snake.turn, 0.0);
        assert!(!snake.input_boost);
    }

    #[test]
    fn grace_never_runs_a_brain_and_missing_takeover_brain_is_explicit() {
        let mut lease = lease();
        let mut snake = snake();
        let disconnect =
            prepare_disconnect(&lease, &snake, 11, 1_100, TIMING).expect("disconnect should stage");
        commit_disconnect(&mut lease, &mut snake, disconnect).expect("disconnect should commit");
        snake.brain = None;

        let neutral = prepare_controller_boundary(&lease, &snake, 20_000, TIMING)
            .expect("brainless grace should remain neutral");
        assert_eq!(neutral.source(), ExternalControlSource::ReservedNeutral);
        assert!(matches!(
            prepare_controller_boundary(&lease, &snake, 31_100, TIMING),
            Err(ControllerError::TakeoverBrainMissing(9))
        ));
    }

    #[test]
    fn proposals_are_atomic_against_intervening_actions() {
        let mut lease = lease();
        let mut snake = snake();
        let boundary = prepare_controller_boundary(&lease, &snake, 1_100, TIMING)
            .expect("boundary should stage");
        let action = prepare_latest_action(&lease, action(-0.75, false, 2, 2, 1_101))
            .expect("new action should stage");
        commit_latest_action(&mut lease, action).expect("new action should commit");
        let original_snake = snake.clone();
        assert!(matches!(
            commit_controller_boundary(&mut lease, &mut snake, boundary),
            Err(ControllerError::StaleProposal { lease_id: 7 })
        ));
        assert_eq!(snake, original_snake);
    }

    #[test]
    fn equal_millisecond_action_sequence_invalidates_an_older_boundary() {
        let mut lease = lease();
        let mut snake = snake();
        let boundary = prepare_controller_boundary(&lease, &snake, 1_000, TIMING)
            .expect("initial boundary should stage");
        let replacement = prepare_latest_action(&lease, action(-0.9, false, 2, 2, 1_000))
            .expect("same-millisecond newer sequence should stage");
        commit_latest_action(&mut lease, replacement)
            .expect("same-millisecond newer sequence should commit");
        let lease_after_action = lease.clone();
        let snake_before_commit = snake.clone();

        assert!(matches!(
            commit_controller_boundary(&mut lease, &mut snake, boundary),
            Err(ControllerError::StaleProposal { lease_id: 7 })
        ));
        assert_eq!(lease, lease_after_action);
        assert_eq!(snake, snake_before_commit);
        let current = prepare_controller_boundary(&lease, &snake, 1_000, TIMING)
            .expect("replacement should be current");
        assert_eq!(
            current.source(),
            ExternalControlSource::HeldAction {
                turn: -0.9,
                boost: false,
            }
        );
        assert!(matches!(
            prepare_latest_action(&lease, action(0.0, false, 3, 2, 1_001)),
            Err(ControllerError::ArrivalSequenceRegressed {
                previous: 2,
                current: 2
            })
        ));
    }

    #[test]
    fn every_committed_boundary_advances_the_lease_wall_clock() {
        let mut lease = lease();
        let mut snake = snake();
        assert!(matches!(
            prepare_controller_boundary(&lease, &snake, 999, TIMING),
            Err(ControllerError::WallClockRegressed {
                previous_ms: 1_000,
                current_ms: 999
            })
        ));

        let boundary = prepare_controller_boundary(&lease, &snake, 1_200, TIMING)
            .expect("later boundary should stage");
        commit_controller_boundary(&mut lease, &mut snake, boundary)
            .expect("later boundary should commit");
        assert_eq!(lease.last_observed_at_ms, 1_200);
        assert!(matches!(
            prepare_controller_boundary(&lease, &snake, 1_199, TIMING),
            Err(ControllerError::WallClockRegressed {
                previous_ms: 1_200,
                current_ms: 1_199
            })
        ));
        assert!(matches!(
            prepare_latest_action(&lease, action(0.1, false, 2, 2, 1_199)),
            Err(ControllerError::WallClockRegressed {
                previous_ms: 1_200,
                current_ms: 1_199
            })
        ));
    }

    #[test]
    fn takeover_revalidates_alive_snake_and_brain_before_any_write() {
        let prepare_takeover = || {
            let mut lease = lease();
            let mut snake = snake();
            let disconnect = prepare_disconnect(&lease, &snake, 11, 1_100, TIMING)
                .expect("disconnect should stage");
            commit_disconnect(&mut lease, &mut snake, disconnect)
                .expect("disconnect should commit");
            let takeover = prepare_controller_boundary(&lease, &snake, 31_100, TIMING)
                .expect("takeover should stage");
            (lease, snake, takeover)
        };

        let (mut dead_lease, mut dead_snake, dead_takeover) = prepare_takeover();
        dead_snake.alive = false;
        let expected_dead_lease = dead_lease.clone();
        let expected_dead_snake = dead_snake.clone();
        assert!(matches!(
            commit_controller_boundary(&mut dead_lease, &mut dead_snake, dead_takeover),
            Err(ControllerError::StaleProposal { lease_id: 7 })
        ));
        assert_eq!(dead_lease, expected_dead_lease);
        assert_eq!(dead_snake, expected_dead_snake);

        let (mut brain_lease, mut brain_snake, brain_takeover) = prepare_takeover();
        brain_snake.brain = None;
        let expected_brain_lease = brain_lease.clone();
        let expected_brain_snake = brain_snake.clone();
        assert!(matches!(
            commit_controller_boundary(&mut brain_lease, &mut brain_snake, brain_takeover),
            Err(ControllerError::StaleProposal { lease_id: 7 })
        ));
        assert_eq!(brain_lease, expected_brain_lease);
        assert_eq!(brain_snake, expected_brain_snake);
    }

    #[test]
    fn disconnect_revalidates_snake_control_state_before_any_write() {
        let mut lease = lease();
        let mut snake = snake();
        let disconnect =
            prepare_disconnect(&lease, &snake, 11, 1_100, TIMING).expect("disconnect should stage");
        snake.turn = -0.5;
        let expected_lease = lease.clone();
        let expected_snake = snake.clone();

        assert!(matches!(
            commit_disconnect(&mut lease, &mut snake, disconnect),
            Err(ControllerError::StaleProposal { lease_id: 7 })
        ));
        assert_eq!(lease, expected_lease);
        assert_eq!(snake, expected_snake);
    }

    #[test]
    fn invalid_timing_deadline_overflow_and_non_external_targets_fail() {
        assert!(ControllerTiming::new(0, 30_000).is_err());
        assert!(ControllerTiming::new(500, 500).is_err());

        let mut overflow = lease();
        overflow.latest_action.accepted_at_ms = u64::MAX - 100;
        overflow.last_observed_at_ms = u64::MAX - 100;
        assert!(matches!(
            prepare_controller_boundary(&overflow, &snake(), u64::MAX, TIMING),
            Err(ControllerError::DeadlineOverflow { .. })
        ));

        let lease = lease();
        let mut evolved = snake();
        evolved.kind = SnakeKind::Evolved;
        assert!(matches!(
            prepare_controller_boundary(&lease, &evolved, 1_100, TIMING),
            Err(ControllerError::SnakeNotExternal(9))
        ));
    }
}
