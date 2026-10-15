//! Bounded presentation from cached committed observations; never a sampling API.

use super::inspector::{
    InspectorObservationSource, RunningInspectorStatus, MAX_INSPECTOR_WIRE_BYTES,
};
use super::sensor_trace::{SensorTrace, SensorTraceRay};
use serde_json::{json, Value};

fn hex(value: u64) -> String {
    format!("{value:016x}")
}

fn trace_json(trace: &SensorTrace) -> Result<Value, String> {
    let bins = trace.bins;
    if !trace.valid || !(8..=32).contains(&bins) {
        return Err("invalid cached inspector trace shape".into());
    }
    let scalar_values = [
        trace.near_radius,
        trace.far_radius,
        trace.age_seconds,
        trace.food,
        trace.fitness,
        trace.size_factor,
        trace.boost_cost,
        trace.boost_headroom,
        trace.generation_seconds,
        trace.food_saturation,
        trace.boost_speed,
        f64::from(trace.held_turn),
        f64::from(trace.previous_applied_turn),
        trace.heading_delta,
        trace.speed_delta,
    ];
    let optional_values = [
        trace.assessment_points,
        trace.life_points,
        trace.generation_best_points,
        trace.generation_elapsed_seconds,
        trace.nearest_food_distance,
        trace.nearest_food_bearing,
        trace.nearest_body_clearance,
        trace.nearest_head_clearance,
    ];
    let arrays = [
        &trace.food_strength,
        &trace.far_food_strength,
        &trace.body_clearance,
        &trace.ray_headings,
        &trace.wall_clearance,
        &trace.head_clearance,
        &trace.head_bearing_offset,
        &trace.head_length_ratio,
        &trace.relative_heading,
        &trace.relative_forward_speed,
        &trace.relative_side_speed,
        &trace.urgency,
    ];
    if scalar_values
        .iter()
        .chain(optional_values.iter().flatten())
        .any(|value| !value.is_finite())
        || arrays
            .iter()
            .any(|values| values[..bins].iter().any(|value| !value.is_finite()))
        || trace.encounter_seconds[..bins]
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        || trace.body_rays[..bins]
            .iter()
            .any(|ray| matches!(ray, SensorTraceRay::Hit(distance) if !distance.is_finite()))
    {
        return Err("non-finite cached inspector trace".into());
    }
    let rays: Vec<_> = trace.body_rays[..bins]
        .iter()
        .map(|ray| match ray {
            SensorTraceRay::NotSampled => json!({"kind":"not-sampled"}),
            SensorTraceRay::Incomplete => json!({"kind":"incomplete"}),
            SensorTraceRay::Clear => json!({"kind":"clear"}),
            SensorTraceRay::Hit(distance) => json!({"kind":"hit","distance":distance}),
        })
        .collect();
    let head_ids: Vec<_> = trace.head_ids[..bins]
        .iter()
        .map(|id| (*id != 0).then(|| hex(*id)))
        .collect();
    let mut projection = json!({
        "nearRadius":trace.near_radius,"farRadius":trace.far_radius,
        "bodyLength":trace.body_length,"ageSeconds":trace.age_seconds,"food":trace.food,
        "kills":hex(trace.kills),"fitness":trace.fitness,"sizeFactor":trace.size_factor,
        "actualBoost":trace.actual_boost,"heldBoostRequest":trace.held_boost_request,
        "heldTurn":trace.held_turn,"previousAppliedTurn":trace.previous_applied_turn,
        "headingDelta":trace.heading_delta,"speedDelta":trace.speed_delta,
        "boostCost":trace.boost_cost,"boostHeadroom":trace.boost_headroom,"boostEligible":trace.boost_eligible,
        "assessmentPoints":trace.assessment_points,"lifePoints":trace.life_points,
        "generationBestPoints":trace.generation_best_points,
        "generationElapsedSeconds":trace.generation_elapsed_seconds,"generationSeconds":trace.generation_seconds,
        "foodSaturation":trace.food_saturation,"boostSpeed":trace.boost_speed,
    });
    let channels = json!({
        "nearestFoodDistance":trace.nearest_food_distance,"nearestFoodBearing":trace.nearest_food_bearing,
        "nearestBodyClearance":trace.nearest_body_clearance,"nearestHeadClearance":trace.nearest_head_clearance,
        "pelletSaturated":trace.pellet_saturated,"bodySaturated":trace.body_saturated,
        "foodStrength":trace.food_strength[..bins],"farFoodStrength":trace.far_food_strength[..bins],
        "bodyClearance":trace.body_clearance[..bins],"bodyRays":rays,"rayHeadings":trace.ray_headings[..bins],
        "wallClearance":trace.wall_clearance[..bins],"headClearance":trace.head_clearance[..bins],
        "headCount":trace.head_count[..bins],"headIds":head_ids,
        "headBearingOffset":trace.head_bearing_offset[..bins],"headLengthRatio":trace.head_length_ratio[..bins],
        "relativeHeading":trace.relative_heading[..bins],"relativeForwardSpeed":trace.relative_forward_speed[..bins],
        "relativeSideSpeed":trace.relative_side_speed[..bins],"urgency":trace.urgency[..bins],
        "encounterSeconds":trace.encounter_seconds[..bins],
    });
    if let (Some(projection), Value::Object(channels)) = (projection.as_object_mut(), channels) {
        projection.extend(channels);
    }
    Ok(projection)
}

/// Serialize one mailbox copy with exact string identities and a hard UTF-8 ceiling.
pub fn encode(snapshot: &RunningInspectorStatus) -> Result<String, String> {
    let sample = snapshot.observation.as_ref().map(|sample| {
        let age = sample.age_steps(snapshot.completed_step).ok_or("inspector sample is newer than publication")?;
        if sample.target.world_epoch != snapshot.world_epoch {
            return Err("inspector world identity mismatch".into());
        }
        let contract = super::sensor_contract::SensorContractIdentity::new(sample.layout.profile, sample.layout.bins)
            .map_err(|error| error.to_string())?;
        if contract.input_size() != sample.normalized_inputs().len()
            || sample.normalized_inputs().iter().any(|value| !value.is_finite()) {
            return Err("invalid inspector normalized row".into());
        }
        let source = match sample.source {
            InspectorObservationSource::Neural => json!({"kind":"neural"}),
            InspectorObservationSource::BaselineV1 => json!({"kind":"baseline-v1"}),
            InspectorObservationSource::BaselineV2 => json!({"kind":"baseline-v2"}),
            InspectorObservationSource::External { lease_id, connection_id } =>
                json!({"kind":"external","leaseId":hex(lease_id),"connectionId":hex(connection_id)}),
        };
        let raw = &sample.raw;
        let kind = match raw.kind {
            super::state::SnakeKind::Evolved => "evolved",
            super::state::SnakeKind::Baseline => "baseline",
            super::state::SnakeKind::External => "external",
            super::state::SnakeKind::Resurrected => "retained-neural",
        };
        let baseline = sample.baseline_action.map(|action| json!({
            "turn":action.turn,"boost":action.boost,"sector":action.sector,
            "lifeStage":match action.life_stage { super::baseline_control::BaselineLifeStage::Small => "small",
                super::baseline_control::BaselineLifeStage::Medium => "medium",super::baseline_control::BaselineLifeStage::Large => "large" },
            "strategy":match action.next_state.strategy { super::state::BaselineStrategyState::Roam => "roam",
                super::state::BaselineStrategyState::Seek => "seek",super::state::BaselineStrategyState::Avoid => "avoid",
                super::state::BaselineStrategyState::Boost => "boost" },
            "normalizedScore":action.normalized_score,"unsafeRoute":action.unsafe_route,
            "escapeOrPursuit":action.escape_or_pursuit,"urgentSectors":action.urgent_sectors,
            "committedHeading":action.next_state.committed_heading,
            "commitmentRemainingSeconds":action.next_state.commitment_remaining_seconds,
        }));
        if sample.trace.as_ref().is_some_and(|trace| trace.physical_id != sample.target.physical_id
            || trace.profile != sample.layout.profile || trace.bins != sample.layout.bins) {
            return Err("inspector trace identity mismatch".into());
        }
        let trace = sample.trace.as_ref().map(trace_json).transpose()?;
        Ok::<_,String>(json!({
            "generation":hex(sample.target.generation),"physicalId":hex(sample.target.physical_id),
            "snakeId":sample.target.frame_v1_id,"populationSlot":raw.population_slot,
            "sampleStep":hex(sample.sample_step().ok_or("inspector step overflow")?),"ageSteps":hex(age),
            "source":source,"profile":sample.layout.profile,"bins":sample.layout.bins,
            "sensorSha256":contract.sha256_hex().map_err(|error| error.to_string())?,
            "normalizedInputs":sample.normalized_inputs(),"neuralOutputs":sample.neural_outputs,"baselineAction":baseline,
            "raw":{"kind":kind,"points":raw.points,"previouslyDeliveredPoints":raw.previously_delivered_points,
                "firstDelivery":raw.first_delivery,"x":raw.x,"y":raw.y,"heading":raw.heading,"radius":raw.radius,
                "speed":raw.speed,"controlAccumulatorSeconds":raw.control_accumulator_seconds},
            "trace":trace,
            "diagnostics":{"pelletChecks":sample.diagnostics.pellet_checks,
                "segmentChecks":sample.diagnostics.segment_checks,"headChecks":sample.diagnostics.head_checks,
                "rayWork":sample.diagnostics.ray_work.charged(),"incompleteRays":sample.diagnostics.incomplete_rays},
        }))
    }).transpose()?;
    let value = json!({"schemaVersion":1,"sequence":hex(snapshot.sequence),
        "worldEpoch":hex(snapshot.world_epoch),"completedStep":hex(snapshot.completed_step),"sample":sample});
    let result = serde_json::to_string(&value).map_err(|error| error.to_string())?;
    if result.len() > MAX_INSPECTOR_WIRE_BYTES {
        return Err("inspector projection exceeds byte admission".into());
    }
    Ok(result)
}
