use super::*;
use crate::engine::contract::ENGINE_CONTRACT_VERSION;
use crate::engine::rng::StatefulRng;
use crate::engine::state::{
    normalized_config_hash, normalized_settings_schema_hash, ALLOCATOR_VERSION,
    BASELINE_ENTITY_ID_START, ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START,
    GENERATION_BOUNDARY_VERSION, NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION,
    RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION, SENSOR_VERSION, SERIALIZER_VERSION,
};
use std::sync::atomic::{AtomicU64, Ordering};

mod fixtures;
use fixtures::*;

mod malformed_archives;
mod numeric_limits;
mod publication_and_recovery;
mod roundtrip_and_versions;
