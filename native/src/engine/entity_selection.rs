//! On-demand physical selection, independent of frame IDs and simulation RNG.
use super::{AuthoritativeState, SnakeKind, StateError};

/// Bounded selector for a current physical entity or persistent population owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntitySelectionTarget {
    Frame(u32),
    PopulationSlot(u32),
}

/// Ephemeral world nonce, process-local world epoch and full physical ID.
/// The nonce is OS entropy and is never checkpointed or drawn from gameplay RNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntitySelectionToken {
    nonce: [u64; 2],
    world_epoch: u64,
    physical_id: u64,
}

impl EntitySelectionToken {
    /// Encode a fixed-size canonical token without narrowing any identity.
    #[must_use]
    pub fn encode(self) -> String {
        format!(
            "{:016x}{:016x}{:016x}{:016x}",
            self.nonce[0], self.nonce[1], self.world_epoch, self.physical_id
        )
    }
    /// Accept only exactly 64 lowercase hex characters and positive identities.
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("entity selection token must be 64 lowercase hex characters".to_owned());
        }
        let mut words = [0_u64; 4];
        for (index, word) in words.iter_mut().enumerate() {
            *word = u64::from_str_radix(&value[index * 16..(index + 1) * 16], 16)
                .map_err(|_| "invalid entity selection token".to_owned())?;
        }
        if words[2] == 0 || words[3] == 0 {
            return Err("entity selection identity must be positive".to_owned());
        }
        Ok(Self {
            nonce: [words[0], words[1]],
            world_epoch: words[2],
            physical_id: words[3],
        })
    }
}

/// Small scalar snapshot for one selected physical entity; no body or weights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntitySelection {
    pub token: EntitySelectionToken,
    pub frame_v1_id: u32,
    pub population_slot: Option<u32>,
    pub life_ordinal: Option<u64>,
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub skin: u32,
}

/// Allocate an ephemeral incarnation without advancing any authoritative stream.
pub(super) fn new_selection_nonce() -> Result<[u64; 2], StateError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| StateError::InvalidField {
        field: "entity_selection.incarnation",
        reason: format!("OS entropy unavailable: {error}"),
    })?;
    Ok([
        u64::from_le_bytes(bytes[..8].try_into().expect("fixed nonce prefix")),
        u64::from_le_bytes(bytes[8..].try_into().expect("fixed nonce suffix")),
    ])
}

impl AuthoritativeState {
    /// Resolve only a live current life. Dead/pending owners return no entity.
    pub(crate) fn select_entity(&self, target: EntitySelectionTarget) -> Option<EntitySelection> {
        let snake = self.candidate.world.snakes.iter().find(|snake| {
            snake.alive
                && match target {
                    EntitySelectionTarget::Frame(frame) => snake.frame_v1_id == frame,
                    EntitySelectionTarget::PopulationSlot(slot) => {
                        snake.kind == SnakeKind::Evolved && snake.population_slot == Some(slot)
                    }
                }
        })?;
        let life_ordinal = snake.population_slot.and_then(|slot| {
            self.candidate
                .fixed_step
                .evolved_generation
                .slots
                .get(slot as usize)
                .map(|life| life.life_ordinal)
                .or(Some(1))
        });
        Some(EntitySelection {
            token: EntitySelectionToken {
                nonce: self.selection_nonce,
                world_epoch: self.world_epoch,
                physical_id: snake.id,
            },
            frame_v1_id: snake.frame_v1_id,
            population_slot: snake.population_slot,
            life_ordinal,
            x: snake.position.x,
            y: snake.position.y,
            radius: snake.radius,
            skin: snake.skin,
        })
    }

    /// Revalidate at the command's clean boundary, before preparing any mutation.
    pub(crate) fn validate_entity_selection(
        &self,
        token: EntitySelectionToken,
        frame: u32,
    ) -> Result<(), String> {
        if token.nonce != self.selection_nonce
            || token.world_epoch != self.world_epoch
            || !self.candidate.world.snakes.iter().any(|snake| {
                snake.alive && snake.id == token.physical_id && snake.frame_v1_id == frame
            })
        {
            return Err(
                "selected entity is no longer the current physical life; select it again"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_tokens_preserve_full_width_ids_and_reject_malformed_inputs() {
        let token = EntitySelectionToken {
            nonce: [u64::MAX, 0],
            world_epoch: u64::MAX,
            physical_id: u64::MAX,
        };
        assert_eq!(EntitySelectionToken::parse(&token.encode()).unwrap(), token);
        for value in [
            "",
            &"0".repeat(64),
            &"A".repeat(64),
            &"é".repeat(32),
            &"f".repeat(63),
            &"f".repeat(65),
        ] {
            assert!(EntitySelectionToken::parse(value).is_err());
        }
    }
}
