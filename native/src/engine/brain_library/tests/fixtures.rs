use super::*;

pub(super) struct TestDirectory(pub(super) PathBuf);
impl TestDirectory {
    pub(super) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "slither-brain-library-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(super) fn source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/portable-brain-v1.slither-brain")
}
pub(super) fn operation() -> CheckpointOperationId {
    CheckpointOperationId::parse("a1".repeat(16)).unwrap()
}
pub(super) fn digest() -> String {
    validate_brain_artifact(&fs::read(source()).unwrap())
        .unwrap()
        .artifact_sha256
}

pub(super) fn admitted_loop(
    cadence: f64,
    profile: crate::engine::sensor_contract::SensorProfile,
) -> crate::engine::running_loop::RunningAuthorityLoop {
    let artifact = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
    admitted_graph_loop(cadence, profile, 8, artifact.graph.spec().clone())
}

pub(super) fn admitted_graph_loop(
    cadence: f64,
    profile: crate::engine::sensor_contract::SensorProfile,
    bins: usize,
    graph: crate::engine::graph::GraphSpec,
) -> crate::engine::running_loop::RunningAuthorityLoop {
    use crate::engine::fresh_run::{
        prepare_v4_qualification_run, FreshRunSettingUpdate, Stage6aP0FreshRunRequest,
    };
    use crate::engine::state::NormalizedSettingValue;
    let settings = crate::engine::step_config::production_default_settings(12, 0)
        .into_iter()
        .filter_map(|setting| match setting.value {
            NormalizedSettingValue::Float(value) => Some(FreshRunSettingUpdate {
                path: setting.path,
                value,
            }),
            NormalizedSettingValue::Integer(value) => Some(FreshRunSettingUpdate {
                path: setting.path,
                value: value as f64,
            }),
            NormalizedSettingValue::Bool(value) => Some(FreshRunSettingUpdate {
                path: setting.path,
                value: f64::from(u8::from(value)),
            }),
            NormalizedSettingValue::Text(_) => None,
        })
        .map(|mut setting| {
            if setting.path == "brain.controlDt" {
                setting.value = cadence;
            }
            setting
        })
        .collect::<Vec<_>>();
    let limits = crate::engine::step_config::RunningStepWorkLimits {
        ray_work: Some(crate::engine::ray_work::RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
        ..crate::engine::step_config::RunningStepWorkLimits::provisional_defaults()
    };
    let mut pending = prepare_v4_qualification_run(
        Stage6aP0FreshRunRequest {
            run_id: "saved-brain-live-test".into(),
            seed: 42,
            memory_ceiling_bytes: 4 * 1024 * 1024 * 1024,
        },
        &settings,
        graph,
        true,
        profile,
        bins,
        limits,
    )
    .unwrap();
    let directory = TestDirectory::new();
    let checkpoint = pending
        .publish_checkpoint(&directory.0, operation())
        .unwrap();
    pending.acknowledge_persistence(&checkpoint).unwrap();
    pending.publish_running_authority().unwrap();
    pending
        .into_running_loop(
            crate::engine::scheduler::FixedStepSchedulerPolicy::provisional_defaults(),
            0,
        )
        .unwrap()
}

/// Build a test-only portable envelope from the actual native compiler/manifest.
pub(super) fn portable_source(directory: &TestDirectory, renamed: bool) -> PathBuf {
    use crate::engine::graph::{GraphBundle, GraphNodeKind};
    use crate::engine::sensor_contract::SensorProfile;
    use crate::engine::sensor_layout::SensorLayout;
    let fixture = validate_brain_artifact(&fs::read(source()).unwrap()).unwrap();
    let mut spec = fixture.graph.spec().clone();
    for node in &mut spec.nodes {
        match &mut node.kind {
            GraphNodeKind::Input { output_size } => *output_size = 144,
            GraphNodeKind::Gru { input_size, .. } => *input_size = 144,
            _ => {}
        }
    }
    if renamed {
        let old = spec.nodes[0].id.clone();
        spec.nodes[0].id = "different-node-identity".into();
        for edge in &mut spec.edges {
            if edge.from == old {
                edge.from = spec.nodes[0].id.clone();
            }
            if edge.to == old {
                edge.to = spec.nodes[0].id.clone();
            }
        }
        for output in &mut spec.outputs {
            if output.node_id == old {
                output.node_id = spec.nodes[0].id.clone();
            }
        }
    }
    let graph =
        GraphBundle::compile(spec, &crate::engine::fresh_run::stage6a_p0_graph_limits()).unwrap();
    let sensors = SensorLayout::v4(SensorProfile::Basic, 20).unwrap();
    let manifest =
        CanonicalTrainingGraphManifestV1::build(&graph, &sensors, fixture.deployment).unwrap();
    let weights = vec![0u8; manifest.total_parameters * 4];
    let digest = Sha256::digest(&weights);
    let json = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1, "graphManifest": serde_json::from_slice::<serde_json::Value>(&manifest.canonical_bytes().unwrap()).unwrap(), "provenance": fixture.provenance
    }))
    .unwrap();
    let mut bytes = vec![0; 88];
    bytes[..8].copy_from_slice(b"SLTHBRN1");
    bytes[8..12].copy_from_slice(&(json.len() as u32).to_le_bytes());
    bytes[16..20].copy_from_slice(&((weights.len() / 4) as u32).to_le_bytes());
    bytes[20..24].copy_from_slice(&(weights.len() as u32).to_le_bytes());
    bytes[24..56].copy_from_slice(&digest);
    bytes[56..88].copy_from_slice(&digest);
    bytes.extend(json);
    bytes.extend(weights);
    validate_brain_artifact(&bytes).unwrap();
    let path = directory.0.join("source.slither-brain");
    fs::write(&path, bytes).unwrap();
    path
}
