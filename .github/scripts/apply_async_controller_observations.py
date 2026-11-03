from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    Path(path).write_text(text, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected one match, found {count}: {old[:120]!r}")
    write(path, text.replace(old, new, 1))


# Native step coordinator: add an allocation-free local acceptance path for
# observation-only batches. Replacement assignments continue using receipts.
path = "native/src/engine/running_step.rs"
needle = '''    /// Apply local Node send results to the one retained external event batch.\n    ///\n    /// Wrong-world, wrong-step, wrong-connection, wrong-assignment, unknown,\n'''
method = '''    /// Publish one retained observation-only batch after its complete output has\n    /// already been admitted by the background reliable queue. Controller\n    /// ownership-changing replacement assignments are deliberately excluded.\n    pub(crate) fn publish_pending_observations<'workspace>(\n        &'workspace mut self,\n        authority: &mut AuthoritativeState,\n    ) -> Result<RunningStepOutcome<'workspace>, RunningStepError> {\n        self.validate_authority(authority)?;\n        let key = self\n            .pending_key\n            .ok_or(RunningStepError::PendingDeliveryStateMismatch { field: "step key" })?;\n        if self.pending_delivery_context != Some(PendingDeliveryContext::RunningStep) {\n            return Err(RunningStepError::PendingDeliveryStateMismatch {\n                field: "observation-only delivery context",\n            });\n        }\n        self.validate_pending_batch(key)?;\n        if self.pending_events.is_empty()\n            || self.pending_events.len() != self.pending_observations.len()\n            || self.pending_observation_statuses.len() != self.pending_observations.len()\n        {\n            return Err(RunningStepError::PendingDeliveryStateMismatch {\n                field: "observation-only retained counts",\n            });\n        }\n        for (index, event) in self.pending_events.iter().enumerate() {\n            if !matches!(event.delivery_kind, ExternalDeliveryEventKind::Observation)\n                || !matches!(\n                    self.pending_sources.get(index),\n                    Some(PendingExternalSource::Observation { retained_index }) if *retained_index == index\n                )\n                || self.pending_statuses[index] != ExternalDeliveryStatus::Pending\n                || self.pending_observation_statuses[index] != ExternalDeliveryStatus::Pending\n            {\n                return Err(RunningStepError::PendingDeliveryStateMismatch {\n                    field: "observation-only event state",\n                });\n            }\n        }\n        self.validate_pending_publication(authority, key)?;\n        self.pending_statuses.fill(ExternalDeliveryStatus::Accepted);\n        self.pending_observation_statuses\n            .fill(ExternalDeliveryStatus::Accepted);\n        let inputs = self\n            .pending_inputs\n            .expect("prevalidated pending observation delivery must retain step inputs");\n        let preflight = self\n            .pending_preflight\n            .expect("prevalidated pending observation delivery must retain state proof");\n        self.world_step.commit_prevalidated_external_deliveries(\n            key,\n            &self.pending_observations,\n            &self.pending_observation_statuses,\n        );\n        let diagnostics = self.world_step.diagnostics();\n        self.publish_prevalidated_staged(authority, key, inputs, preflight, diagnostics)\n    }\n\n'''
text = read(path)
if needle not in text:
    raise RuntimeError("running_step insertion point missing")
write(path, text.replace(needle, method + needle, 1))

# Running loop: retire the retained scheduler ticket locally once a complete
# observation-only output batch has been owned and reserved by the coordinator.
path = "native/src/engine/running_loop.rs"
needle = '''    /// Apply exact local-send results to the one retained ordinary-step or\n    /// generation-assignment batch.\n'''
method = '''    /// Publish a retained observation-only step after the background output\n    /// queue has reserved and fully owned its sensor batch. This deliberately\n    /// cannot resolve replacement assignments or generation handoffs.\n    pub(crate) fn publish_pending_observations(\n        &mut self,\n        frame: Option<RunningFramePublication<'_>>,\n    ) -> Result<RunningAuthorityLoopProgress, RunningAuthorityLoopError> {\n        self.require_action_state(\n            "publish pending controller observations",\n            RunningAuthorityLoopState::ExternalDeliveryPending,\n        )?;\n        let step = self.validate_retained_step()?;\n        let due_steps = self\n            .pending_due_steps\n            .expect("validated retained step must include due-step count");\n        let outcome = match self\n            .coordinator\n            .publish_pending_observations(&mut self.authority)\n        {\n            Ok(outcome) => outcome,\n            Err(error) => {\n                self.state = RunningAuthorityLoopState::Faulted;\n                return Err(error.into());\n            }\n        };\n        let publication = outcome.publication;\n        self.capture_teacher_publication(false);\n        if let Err(error) = self.scheduler.commit_step(&self.authority, step, publication) {\n            self.state = RunningAuthorityLoopState::Faulted;\n            return Err(error.into());\n        }\n        self.pending_step = None;\n        self.pending_due_steps = None;\n        let frame = match frame {\n            Some(publication_request) => match pack_authoritative_frame_v1_into(\n                &self.authority,\n                publication_request.view,\n                publication_request.output,\n            ) {\n                Ok(metadata) => Some(metadata),\n                Err(error) => {\n                    self.state = RunningAuthorityLoopState::Faulted;\n                    return Err(error.into());\n                }\n            },\n            None => None,\n        };\n        self.state = RunningAuthorityLoopState::Ready;\n        Ok(RunningAuthorityLoopProgress::Published {\n            ticket_sequence: step.sequence(),\n            due_steps,\n            publication,\n            frame,\n        })\n    }\n\n'''
text = read(path)
if needle not in text:
    raise RuntimeError("running_loop insertion point missing")
write(path, text.replace(needle, method + needle, 1))

# Contract: distinguish post-publication observations from receipt-driven
# ordinary replacement batches at the Rust/Node bridge.
path = "native/src/engine/contract.rs"
replace_once(path,
'''    /// The entire ordinary-step delivery batch, admitted before step preparation.\n    ControllerMessages {\n        ticket_sequence: u64,\n        messages: Box<[RunningControllerMessage]>,\n    },\n''',
'''    /// Ordered ordinary observations whose authoritative step is already published.\n    /// Their complete payload was admitted to the bounded Rust output queue before\n    /// the step committed, so Node must not return ordinary delivery receipts.\n    ControllerObservations {\n        ticket_sequence: u64,\n        messages: Box<[RunningControllerMessage]>,\n    },\n    /// Receipt-driven ordinary-step batch containing an ownership-changing replacement.\n    ControllerMessages {\n        ticket_sequence: u64,\n        messages: Box<[RunningControllerMessage]>,\n    },\n''')
replace_once(path,
'''            Self::ControllerMessages { messages, .. } => messages\n                .len()\n                .saturating_mul(size_of::<RunningControllerMessage>())\n                .saturating_add(messages.iter().fold(0usize, |bytes, message| {\n                    bytes.saturating_add(message.payload_owned_bytes())\n                })),\n''',
'''            Self::ControllerObservations { messages, .. }\n            | Self::ControllerMessages { messages, .. } => messages\n                .len()\n                .saturating_mul(size_of::<RunningControllerMessage>())\n                .saturating_add(messages.iter().fold(0usize, |bytes, message| {\n                    bytes.saturating_add(message.payload_owned_bytes())\n                })),\n''')

# Coordinator: while the output reservation is held, own the complete sensor
# payload, preflight its exact byte size, locally publish the observation-only
# step, then enqueue the already-owned event. Replacement batches retain the
# old blocked receipt path.
path = "native/src/engine/coordinator.rs"
replace_once(path,
'''        let progress = match running.service_after_command_drain(\n            monotonic_elapsed_ms(wall_origin)?,\n            SchedulerServiceMode::Background,\n            None,\n        ) {\n''',
'''        let mut progress = match running.service_after_command_drain(\n            monotonic_elapsed_ms(wall_origin)?,\n            SchedulerServiceMode::Background,\n            None,\n        ) {\n''')
needle = '''        if was_ready\n            && matches!(\n                progress,\n'''
insert = '''        let mut asynchronous_observations = None;\n        if let RunningAuthorityLoopProgress::ExternalDeliveryPending {\n            ticket_sequence, ..\n        } = progress\n        {\n            let observation_only = running.pending_external_delivery().is_some_and(|batch| {\n                !batch.events().is_empty()\n                    && batch.events().iter().all(|event| {\n                        matches!(event.delivery_kind, ExternalDeliveryEventKind::Observation)\n                    })\n            });\n            if observation_only {\n                if service_reservation.is_none() {\n                    return Err(EngineError::new(\n                        EngineErrorCode::Faulted,\n                        "ordinary controller observations lost their admitted output reservation",\n                    ));\n                }\n                let messages = super::controller_output::own_pending_messages(running)?;\n                let event = RunningAuthorityEvent::ControllerObservations {\n                    ticket_sequence,\n                    messages,\n                };\n                if event.owned_bytes() > service_reply_bytes {\n                    return Err(EngineError::new(\n                        EngineErrorCode::QueueByteLimit,\n                        "controller observation output exceeded its pre-step reservation",\n                    ));\n                }\n                progress = running\n                    .publish_pending_observations(None)\n                    .map_err(|error| EngineError::new(EngineErrorCode::Faulted, error.to_string()))?;\n                asynchronous_observations = Some(event);\n            }\n        }\n'''
text = read(path)
if needle not in text:
    raise RuntimeError("coordinator async insertion point missing")
write(path, text.replace(needle, insert + needle, 1))
replace_once(path,
'''            RunningAuthorityLoopProgress::Published { .. } => {\n                announced_generation_source = None;\n                announced_delivery_ticket = None;\n                RunningWait::Immediate\n            }\n''',
'''            RunningAuthorityLoopProgress::Published { .. } => {\n                if let Some(event) = asynchronous_observations.take() {\n                    let reservation = service_reservation.ok_or_else(|| {\n                        EngineError::new(\n                            EngineErrorCode::Faulted,\n                            "published controller observations lost their admitted output reservation",\n                        )\n                    })?;\n                    reservation.publish(ReliableEvent::RunningAuthority(Box::new(event)))?;\n                }\n                announced_generation_source = None;\n                announced_delivery_ticket = None;\n                RunningWait::Immediate\n            }\n''')

# N-API event surface: add a separate controllerObservations field and map the
# new event without changing the Protocol 2 WebSocket object.
path = "native/src/napi_engine.rs"
replace_once(path,
'''    pub controller_messages: Option<Vec<BackgroundControllerMessage>>,\n    pub controller_receipt_resolution: Option<BackgroundControllerReceiptResolution>,\n''',
'''    pub controller_observations: Option<Vec<BackgroundControllerMessage>>,\n    pub controller_messages: Option<Vec<BackgroundControllerMessage>>,\n    pub controller_receipt_resolution: Option<BackgroundControllerReceiptResolution>,\n''')
replace_once(path,
'''        controller_messages: None,\n        controller_receipt_resolution: None,\n''',
'''        controller_observations: None,\n        controller_messages: None,\n        controller_receipt_resolution: None,\n''')
old = '''        RunningAuthorityEvent::ControllerMessages { messages, .. } => {\n            output.kind = "controllerMessages".to_owned();\n            output.controller_messages = Some(\n                messages\n                    .into_vec()\n                    .into_iter()\n                    .map(background_controller_message_to_napi)\n                    .collect::<Result<Vec<_>>>()?,\n            );\n        }\n'''
new = '''        RunningAuthorityEvent::ControllerObservations { messages, .. } => {\n            output.kind = "controllerObservations".to_owned();\n            output.controller_observations = Some(\n                messages\n                    .into_vec()\n                    .into_iter()\n                    .map(background_controller_message_to_napi)\n                    .collect::<Result<Vec<_>>>()?,\n            );\n        }\n        RunningAuthorityEvent::ControllerMessages { messages, .. } => {\n            output.kind = "controllerMessages".to_owned();\n            output.controller_messages = Some(\n                messages\n                    .into_vec()\n                    .into_iter()\n                    .map(background_controller_message_to_napi)\n                    .collect::<Result<Vec<_>>>()?,\n            );\n        }\n'''
replace_once(path, old, new)

# TypeScript bridge contract.
path = "src/protocol/rustBackground.ts"
replace_once(path,
'''  /** Full reliable ordinary-step batch, delivered before its step is published. */\n  controllerMessages?: RustBackgroundControllerMessage[];\n''',
'''  /** Ordered ordinary observations from an already-published authoritative step. */\n  controllerObservations?: RustBackgroundControllerMessage[];\n  /** Receipt-driven ordinary-step batch containing a controller replacement. */\n  controllerMessages?: RustBackgroundControllerMessage[];\n''')

# Node delivery adapter: add a no-receipt observation router. It uses the same
# strict payload validation as the transactional adapter.
path = "server/rustEngine/controllerDelivery.ts"
needle = '''/**\n * Bounded adapter for one reliable ordinary-step batch. The event pump must stop\n'''
addition = '''/** Transport for ordered observations whose authoritative step is already committed. */\nexport interface ControllerObservationPorts {\n  /** Current Rust-admitted sensor width. */\n  sensorCount?(): number;\n  /** Route to the exact socket epoch; false means local transport rejected it. */\n  send(connectionId: RustBackgroundIdentity, message: SensorsMsg): boolean;\n  /** Report only an exact failed lease/connection so Rust can disconnect asynchronously. */\n  failed(connectionId: RustBackgroundIdentity, leaseId: RustBackgroundIdentity): void;\n}\n\n/** Send already-published observation batches without manufacturing native receipts. */\nexport class ControllerObservationRouter {\n  constructor(private readonly maxMessages: number, private readonly ports: ControllerObservationPorts) {\n    if (!Number.isSafeInteger(maxMessages) || maxMessages <= 0) {\n      throw new RangeError('controller observation capacity must be a positive safe integer');\n    }\n  }\n\n  /** Validate the complete batch before the first externally visible send. */\n  deliver(messages: readonly RustBackgroundControllerMessage[]): void {\n    if (messages.length === 0 || messages.length > this.maxMessages) {\n      throw new RangeError('controller observation batch exceeds admitted message capacity');\n    }\n    if (messages.some(message => message.kind !== 'observation')) {\n      throw new TypeError('asynchronous controller output may contain observations only');\n    }\n    const sensorCount = this.ports.sensorCount?.();\n    const prepared = messages.map(message => prepare(message, sensorCount));\n    for (const item of prepared) {\n      if (item.message.type !== 'sensors') {\n        throw new TypeError('asynchronous controller output prepared a non-observation');\n      }\n    }\n    for (let index = 0; index < prepared.length; index++) {\n      const item = prepared[index]!;\n      const source = messages[index]!;\n      let accepted = false;\n      try { accepted = this.ports.send(source.connectionId, item.message as SensorsMsg); }\n      catch { accepted = false; }\n      if (!accepted) this.ports.failed(source.connectionId, source.leaseId);\n    }\n  }\n}\n\n'''
text = read(path)
if needle not in text:
    raise RuntimeError("controllerDelivery insertion point missing")
write(path, text.replace(needle, addition + needle, 1))

# Background output pump: observations never retain/flush receipts.
path = "server/rustEngine/backgroundOutput.ts"
replace_once(path,
'''import { ControllerDeliveryRouter, JoinDeliveryRouter, ReclaimDeliveryRouter } from './controllerDelivery.ts';\n''',
'''import { ControllerDeliveryRouter, ControllerObservationRouter, JoinDeliveryRouter, ReclaimDeliveryRouter } from './controllerDelivery.ts';\n''')
replace_once(path,
'''  /** Observe scalar routing/lifecycle results before delivery of their messages. */\n  event(event: RustBackgroundEvent): void;\n''',
'''  /** Observe scalar routing/lifecycle results before delivery of their messages. */\n  event(event: RustBackgroundEvent): void;\n  /** Disconnect only the exact current assignment after an asynchronous observation send fails. */\n  observationDeliveryFailed?(connectionId: RustBackgroundIdentity, leaseId: RustBackgroundIdentity): void;\n''')
replace_once(path,
'''  /** Retained ordinary batch receipts. */\n  private readonly ordinary: ControllerDeliveryRouter;\n''',
'''  /** Ordered post-publication observations; this path owns no native receipts. */\n  private readonly observations: ControllerObservationRouter;\n  /** Retained transactional ordinary batch receipts. */\n  private readonly ordinary: ControllerDeliveryRouter;\n''')
replace_once(path,
'''    this.admission = new BackgroundCommandAdmission(owner.runtime);\n    this.ordinary = new ControllerDeliveryRouter(maxControllers, {\n''',
'''    this.admission = new BackgroundCommandAdmission(owner.runtime);\n    this.observations = new ControllerObservationRouter(maxControllers, {\n      sensorCount: () => this.currentSensorCount(),\n      send: (connectionId, message) => send(connectionId, message),\n      failed: (connectionId, leaseId) => options.observationDeliveryFailed?.(connectionId, leaseId)\n    });\n    this.ordinary = new ControllerDeliveryRouter(maxControllers, {\n''')
replace_once(path,
'''        if (await this.generation.handle(event)) continue;\n        if (event.controllerMessages && !this.ordinary.deliver(event.controllerMessages)) {\n''',
'''        if (await this.generation.handle(event)) continue;\n        if (event.controllerObservations) this.observations.deliver(event.controllerObservations);\n        if (event.controllerMessages && !this.ordinary.deliver(event.controllerMessages)) {\n''')

# External route: an already-published observation send failure is converted to
# the existing exact native disconnect command, guarded by both connection and lease.
path = "server/rustEngine/externalRouting.ts"
needle = '''  /** Update socket routing tags solely from native envelopes and correlated results. */\n  event(event: RustBackgroundEvent): void {\n'''
addition = '''  /** Disconnect only if an asynchronous observation failure still names the current lease. */\n  observationDeliveryFailed(connectionId: RustBackgroundIdentity, leaseId: RustBackgroundIdentity): void {\n    if (!/^[0-9a-f]{16}$/u.test(connectionId) || !/^[0-9a-f]{16}$/u.test(leaseId)) return;\n    const numeric = Number(BigInt(`0x${connectionId}`));\n    const route = this.routes.get(numeric);\n    if (!route || route.closed || route.id !== connectionId || route.lease !== leaseId) return;\n    route.closed = true;\n    delete route.action;\n    route.pending = 'close';\n    this.flush();\n  }\n\n'''
text = read(path)
if needle not in text:
    raise RuntimeError("externalRouting insertion point missing")
write(path, text.replace(needle, addition + needle, 1))

# Wire the exact failure callback in the production server.
path = "server/rustServer.ts"
replace_once(path,
'''      owner, metadata: () => activeMetadata, maxControllers: MAX_CONTROLLERS,\n      send: (id, message) => sockets.sendJsonTo(Number(BigInt(`0x${id}`)), message),\n      event(event) {\n''',
'''      owner, metadata: () => activeMetadata, maxControllers: MAX_CONTROLLERS,\n      send: (id, message) => sockets.sendJsonTo(Number(BigInt(`0x${id}`)), message),\n      observationDeliveryFailed: (connectionId, leaseId) =>\n        routing.observationDeliveryFailed(connectionId, leaseId),\n      event(event) {\n''')

# TS unit coverage for the new send-without-receipt path.
path = "server/rustEngine/controllerDelivery.test.ts"
replace_once(path,
'''import { ControllerDeliveryRouter, ReclaimDeliveryRouter, GenerationDeliveryRouter } from './controllerDelivery.ts';\n''',
'''import { ControllerDeliveryRouter, ControllerObservationRouter, ReclaimDeliveryRouter, GenerationDeliveryRouter } from './controllerDelivery.ts';\n''')
needle = '''describe('Rust ordinary controller transport', () => {\n'''
tests = '''describe('Rust asynchronous observation transport', () => {\n  it('sends ordered observations without native receipts', () => {\n    const sent: unknown[] = [];\n    const failed: Array<[string, string]> = [];\n    const router = new ControllerObservationRouter(4, {\n      sensorCount: () => 83,\n      send(connectionId, message) { sent.push({ connectionId, message }); return true; },\n      failed(connectionId, leaseId) { failed.push([connectionId, leaseId]); }\n    });\n    const first = observation('0000000000000009');\n    const second = { ...observation('000000000000000a'), sourceCompletedStep: '0000000000000001' };\n    router.deliver([first, second]);\n    expect(sent).toEqual([\n      { connectionId: first.connectionId, message: { type: 'sensors', tick: 0, snakeId: 12, sensors: first.sensors, meta: { x: 5, y: 8, dir: 0.25 } } },\n      { connectionId: second.connectionId, message: { type: 'sensors', tick: 1, snakeId: 12, sensors: second.sensors, meta: { x: 5, y: 8, dir: 0.25 } } }\n    ]);\n    expect(failed).toEqual([]);\n  });\n\n  it('reports exact send failures and rejects replacement messages before sending', () => {\n    const failed: Array<[string, string]> = [];\n    let sends = 0;\n    const router = new ControllerObservationRouter(2, {\n      send() { sends++; return false; },\n      failed(connectionId, leaseId) { failed.push([connectionId, leaseId]); }\n    });\n    const item = observation();\n    router.deliver([item]);\n    expect(failed).toEqual([[item.connectionId, item.leaseId]]);\n    expect(sends).toBe(1);\n    expect(() => router.deliver([{ ...item, kind: 'replacementAssignment', resumeToken: 'token' }])).toThrow('observations only');\n    expect(sends).toBe(1);\n  });\n});\n\n'''
text = read(path)
if needle not in text:
    raise RuntimeError("controllerDelivery test insertion point missing")
write(path, text.replace(needle, tests + needle, 1))

# Durable architecture documentation.
adr = Path("docs/decisions/0004-async-controller-observations.md")
adr.write_text('''# ADR 0004: Ordinary external observations use asynchronous bounded output\n\n- Status: accepted by owner direction\n- Decision date: 2026-11-03\n\n## Context\n\nThe Rust-authoritative runtime originally treated every ordinary external sensor observation like an ownership-changing controller assignment. A fixed step retained its scheduler ticket until Node attempted each WebSocket send and returned local delivery receipts to Rust. With several PPO controllers this inserted a serial Rust-to-Node-to-Rust barrier after every fixed step, leaving the calculation workers idle even though the observation itself had already been computed.\n\n## Decision\n\nOrdinary observation-only controller output is an ordered asynchronous stream. Before a fixed step mutates authority, the background coordinator waits for and reserves enough bounded Rust reliable-output capacity for the complete possible controller result. It then fully owns and validates the observation payload. Once that payload is proven to fit the reserved slot, Rust commits the observation delivery markers, authoritative step and scheduler ticket without waiting for Node. The already-owned observation event is then published into the reserved Rust output slot.\n\nA failed later local WebSocket admission is handled as an exact asynchronous disconnect using the original connection and lease identities. Stale failures cannot disconnect a newer lease. Sensor observations remain FIFO and are never silently coalesced or replaced. Bounded Rust output and bounded per-socket Node queues remain the backpressure mechanism.\n\nController ownership-changing operations remain receipt-driven transactions. This includes fresh joins, same-snake reclaims, controlled-death replacement assignments, generation-start assignments and generation publication. A physical step containing a replacement assignment keeps the existing ordinary delivery barrier for the complete batch.\n\nProtocol 2 does not change. External clients still receive the same `sensors` JSON object and continue to send fire-and-forget held actions. The internal Rust/Node bridge distinguishes already-published `controllerObservations` from receipt-driven `controllerMessages`.\n\n`points_delta_norm` and external inspector rows use admission to the bounded authoritative observation stream as the delivery boundary. The sensor sample remains the same stable pre-movement observation of that fixed step.\n\n## Consequences\n\n- Routine PPO traffic no longer blocks every authoritative step on Node delivery receipts.\n- Output memory stays bounded; a sufficiently slow consumer eventually creates backpressure or disconnects instead of creating an unbounded backlog.\n- Controlled replacement and generation ownership semantics retain their existing exact local-send receipts.\n- JSON serialization remains on Node and may become the next measurable bottleneck at larger controller counts; changing Protocol 2 encoding is a separate decision.\n''', encoding="utf-8")

# README, API and developer invariant updates use concise additions instead of
# rewriting historical performance records.
path = "README.md"
replace_once(path,
'''It parallelizes sensing and brain evaluation while\nkeeping brain-state and physics commits ordered.\n''',
'''It parallelizes sensing and brain evaluation while\nkeeping brain-state and physics commits ordered. Ordinary external-controller\nsensor observations are published through a bounded asynchronous output stream,\nso healthy Protocol 2 bots do not insert a Node delivery receipt barrier after\nevery fixed step. Ownership-changing assignments remain reliable transactions.\n''')

path = "docs/API-instructions.md"
replace_once(path,
'''The example sensor array is abbreviated. Use `welcome.sensorSpec.sensorCount`\nand `welcome.sensorSpec.order`; do not hard-code the default length. `meta` is\noptional in the type but the current server includes the controlled snake's\npose.\n''',
'''The example sensor array is abbreviated. Use `welcome.sensorSpec.sensorCount`\nand `welcome.sensorSpec.order`; do not hard-code the default length. `meta` is\noptional in the type but the current server includes the controlled snake's\npose.\n\nOrdinary sensor packets form an ordered asynchronous stream. Rust commits the\nfixed step only after the complete observation batch has been admitted to its\nbounded output queue, then continues simulation without waiting for Node, the\nWebSocket, or a client action to acknowledge that observation. `tick` identifies\nthe committed source boundary. The latest accepted action remains held under the\nnormal input timeout. Fresh/replacement assignments, reconnects and generation\nreassignments retain their reliable transactional delivery rules.\n''')

path = "AGENTS.md"
replace_once(path,
'''All due controls use a stable observation boundary before movement, followed by collision-only substeps. Delivered score-delta observations accumulate between deliveries; baseline strategy probes do not consume that boundary.\n''',
'''All due controls use a stable observation boundary before movement, followed by collision-only substeps. Delivered score-delta observations accumulate between deliveries; baseline strategy probes do not consume that boundary. Ordinary external observations commit their delivery marker when the complete ordered batch is admitted to the bounded Rust output stream; they never wait on a per-tick Node receipt and must not be silently coalesced or dropped. Later transport failure disconnects only the exact matching connection/lease. Ownership-changing join, reclaim, replacement and generation assignments remain receipt-driven transactions.\n''')

print("async controller observation patch applied")
