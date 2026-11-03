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


def replace_optional(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count > 1:
        raise RuntimeError(f"{path}: expected at most one match, found {count}: {old[:120]!r}")
    if count == 1:
        write(path, text.replace(old, new, 1))


# The pre-publication proof must cover the full ReliableEvent, including its
# outer running-authority allocation, because that is what the reservation owns.
path = "native/src/engine/coordinator.rs"
replace_once(path,
'''                let event = RunningAuthorityEvent::ControllerObservations {
                    ticket_sequence,
                    messages,
                };
                if event.owned_bytes() > service_reply_bytes {
                    return Err(EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "controller observation output exceeded its pre-step reservation",
                    ));
                }
                progress = running
                    .publish_pending_observations(None)
                    .map_err(|error| {
                        EngineError::new(EngineErrorCode::Faulted, error.to_string())
                    })?;
                asynchronous_observations = Some(event);
''',
'''                let event = ReliableEvent::RunningAuthority(Box::new(
                    RunningAuthorityEvent::ControllerObservations {
                        ticket_sequence,
                        messages,
                    },
                ));
                if event.owned_bytes() > service_reply_bytes {
                    return Err(EngineError::new(
                        EngineErrorCode::QueueByteLimit,
                        "controller observation output exceeded its pre-step reservation",
                    ));
                }
                progress = running
                    .publish_pending_observations(None)
                    .map_err(|error| {
                        EngineError::new(EngineErrorCode::Faulted, error.to_string())
                    })?;
                asynchronous_observations = Some(event);
''')
replace_once(path,
'''                    reservation.publish(ReliableEvent::RunningAuthority(Box::new(event)))?;
''',
'''                    reservation.publish(event)?;
''')

# Native regression for the new transaction boundary. The complete observation
# event is owned and proven to fit first, then the authoritative step can retire
# with no Node receipt and the already-owned event is published into the reserve.
needle = '''    /// Pending vectors, rejected queue admission and terminal batches cannot enter the cached prefix.
    #[test]
    fn observation_quality_counts_only_ordinary_publication_and_never_changes_continuation() {
'''
test = '''    #[test]
    fn observation_only_step_publishes_after_bounded_output_admission_without_receipts() {
        let mut running = ordinary_controller_loop();
        let before_step = running.completed_step();
        let bound = super::super::controller_output::service_reply_bound(&running).unwrap();
        let output = output_with_cap(bound);
        let reservation = output.reserve_authority_reply(bound).unwrap();

        let progress = running
            .service_after_command_drain(1200, SchedulerServiceMode::Background, None)
            .unwrap();
        let ticket_sequence = match progress {
            RunningAuthorityLoopProgress::ExternalDeliveryPending {
                ticket_sequence, ..
            } => ticket_sequence,
            other => panic!("expected retained external observations: {other:?}"),
        };
        let batch = running.pending_external_delivery().unwrap();
        assert!(!batch.events().is_empty());
        assert!(batch.events().iter().all(|event| matches!(
            event.delivery_kind,
            ExternalDeliveryEventKind::Observation
        )));

        let messages = super::super::controller_output::own_pending_messages(&running).unwrap();
        assert_eq!(messages.len(), batch.events().len());
        let event = ReliableEvent::RunningAuthority(Box::new(
            RunningAuthorityEvent::ControllerObservations {
                ticket_sequence,
                messages,
            },
        ));
        assert!(event.owned_bytes() <= bound);

        let publication = running.publish_pending_observations(None).unwrap();
        assert!(matches!(
            publication,
            RunningAuthorityLoopProgress::Published { .. }
        ));
        assert_eq!(running.state(), RunningAuthorityLoopState::Ready);
        assert_eq!(running.completed_step(), before_step + 1);
        assert!(running.pending_external_delivery().is_none());

        reservation.publish(event).unwrap();
        let drained = output.drain(8, bound);
        assert!(drained.events.iter().any(|event| matches!(
            event,
            CompletedEvent::Reliable(ReliableEvent::RunningAuthority(inner))
                if matches!(inner.as_ref(), RunningAuthorityEvent::ControllerObservations { .. })
        )));
    }

    /// Pending vectors, rejected queue admission and terminal batches cannot enter the cached prefix.
    #[test]
    fn observation_quality_counts_only_ordinary_publication_and_never_changes_continuation() {
'''
text = read(path)
if needle not in text:
    raise RuntimeError("coordinator regression insertion point missing")
write(path, text.replace(needle, test, 1))

# The hot asynchronous path should not manufacture receipt objects that can
# never be returned. Transactional replacement batches still use prepare().
path = "server/rustEngine/controllerDelivery.ts"
old = '''/** Translate a complete Rust envelope without calculating sensors or looking up game state. */
function prepare(message: RustBackgroundControllerMessage, expectedSensorCount?: number): PendingDelivery {
  const receipt: RustGenerationAssignmentReceipt = {
    operationEpoch: identity(message.operationEpoch),
    eventSequence: identity(message.eventSequence),
    connectionId: identity(message.connectionId),
    leaseId: identity(message.leaseId),
    accepted: false
  };
  if (!Number.isInteger(message.snakeId) || message.snakeId <= 0 || message.snakeId > 16_777_216) {
    throw new RangeError('controller snake ID exceeds the frame-v1 identity range');
  }
  if (message.controllerKind !== 'player' && message.controllerKind !== 'reinforcementLearning') {
    throw new TypeError('invalid Rust controller kind');
  }
  if (message.kind === 'replacementAssignment') {
    if (typeof message.resumeToken !== 'string' || message.resumeToken.length === 0) {
      throw new TypeError('controller assignment omits its resume token');
    }
    const replacementReason = (message as ReplacementControllerMessage).replacementReason;
    if (replacementReason !== undefined && replacementReason !== 'generation') {
      throw new TypeError('invalid controller replacement reason');
    }
    const assignment: ReplacementAssignMsg = {
      type: 'assign', snakeId: message.snakeId,
      controller: message.controllerKind === 'player' ? 'player' : 'bot',
      resumeToken: message.resumeToken
    };
    if (replacementReason !== undefined) assignment.replacementReason = replacementReason;
    return { receipt, message: assignment };
  }
  if (message.kind !== 'observation' || !/^[0-9a-f]{16}$/u.test(message.sourceCompletedStep)) {
    throw new TypeError('invalid Rust controller observation');
  }
  const tick = BigInt(`0x${message.sourceCompletedStep}`);
  if (tick > MAX_WIRE_INTEGER) throw new RangeError('observation tick exceeds Protocol 2 exact integers');
  const { sensors, x, y, direction } = message;
  if (!Array.isArray(sensors) || !isSupportedSensorCount(sensors.length) ||
      (expectedSensorCount === undefined ? (sensors.length<51 || sensors.length>147 || (sensors.length-19)%4!==0) :
        (!isSupportedSensorCount(expectedSensorCount) || sensors.length!==expectedSensorCount)) || !sensors.every(Number.isFinite) ||
      typeof x !== 'number' || !Number.isFinite(x) || typeof y !== 'number' || !Number.isFinite(y) ||
      typeof direction !== 'number' || !Number.isFinite(direction)) {
    throw new TypeError('controller observation omits the admitted sensor values or pose');
  }
  return { receipt, message: {
    type: 'sensors', tick: Number(tick), snakeId: message.snakeId,
    sensors, meta: { x, y, dir: direction }
  } };
}
'''
new = '''/** Translate one Rust envelope without calculating sensors or looking up game state. */
function prepareMessage(
  message: RustBackgroundControllerMessage,
  expectedSensorCount?: number
): AssignMsg | SensorsMsg {
  identity(message.operationEpoch);
  identity(message.eventSequence);
  identity(message.connectionId);
  identity(message.leaseId);
  if (!Number.isInteger(message.snakeId) || message.snakeId <= 0 || message.snakeId > 16_777_216) {
    throw new RangeError('controller snake ID exceeds the frame-v1 identity range');
  }
  if (message.controllerKind !== 'player' && message.controllerKind !== 'reinforcementLearning') {
    throw new TypeError('invalid Rust controller kind');
  }
  if (message.kind === 'replacementAssignment') {
    if (typeof message.resumeToken !== 'string' || message.resumeToken.length === 0) {
      throw new TypeError('controller assignment omits its resume token');
    }
    const replacementReason = (message as ReplacementControllerMessage).replacementReason;
    if (replacementReason !== undefined && replacementReason !== 'generation') {
      throw new TypeError('invalid controller replacement reason');
    }
    const assignment: ReplacementAssignMsg = {
      type: 'assign', snakeId: message.snakeId,
      controller: message.controllerKind === 'player' ? 'player' : 'bot',
      resumeToken: message.resumeToken
    };
    if (replacementReason !== undefined) assignment.replacementReason = replacementReason;
    return assignment;
  }
  if (message.kind !== 'observation' || !/^[0-9a-f]{16}$/u.test(message.sourceCompletedStep)) {
    throw new TypeError('invalid Rust controller observation');
  }
  const tick = BigInt(`0x${message.sourceCompletedStep}`);
  if (tick > MAX_WIRE_INTEGER) throw new RangeError('observation tick exceeds Protocol 2 exact integers');
  const { sensors, x, y, direction } = message;
  if (!Array.isArray(sensors) || !isSupportedSensorCount(sensors.length) ||
      (expectedSensorCount === undefined ? (sensors.length<51 || sensors.length>147 || (sensors.length-19)%4!==0) :
        (!isSupportedSensorCount(expectedSensorCount) || sensors.length!==expectedSensorCount)) || !sensors.every(Number.isFinite) ||
      typeof x !== 'number' || !Number.isFinite(x) || typeof y !== 'number' || !Number.isFinite(y) ||
      typeof direction !== 'number' || !Number.isFinite(direction)) {
    throw new TypeError('controller observation omits the admitted sensor values or pose');
  }
  return {
    type: 'sensors', tick: Number(tick), snakeId: message.snakeId,
    sensors, meta: { x, y, dir: direction }
  };
}

/** Add a retained receipt only for delivery paths whose success changes authority state. */
function prepare(message: RustBackgroundControllerMessage, expectedSensorCount?: number): PendingDelivery {
  return {
    receipt: {
      operationEpoch: identity(message.operationEpoch),
      eventSequence: identity(message.eventSequence),
      connectionId: identity(message.connectionId),
      leaseId: identity(message.leaseId),
      accepted: false
    },
    message: prepareMessage(message, expectedSensorCount)
  };
}
'''
replace_once(path, old, new)
replace_once(path,
'''    const sensorCount = this.ports.sensorCount?.();
    const prepared = messages.map(message => prepare(message, sensorCount));
    for (const item of prepared) {
      if (item.message.type !== 'sensors') {
        throw new TypeError('asynchronous controller output prepared a non-observation');
      }
    }
    for (let index = 0; index < prepared.length; index++) {
      const item = prepared[index]!;
      const source = messages[index]!;
      let accepted = false;
      try { accepted = this.ports.send(source.connectionId, item.message as SensorsMsg); }
      catch { accepted = false; }
      if (!accepted) this.ports.failed(source.connectionId, source.leaseId);
    }
''',
'''    const sensorCount = this.ports.sensorCount?.();
    const prepared = messages.map(message => prepareMessage(message, sensorCount));
    for (const item of prepared) {
      if (item.type !== 'sensors') {
        throw new TypeError('asynchronous controller output prepared a non-observation');
      }
    }
    for (let index = 0; index < prepared.length; index++) {
      const item = prepared[index]! as SensorsMsg;
      const source = messages[index]!;
      let accepted = false;
      try { accepted = this.ports.send(source.connectionId, item); }
      catch { accepted = false; }
      if (!accepted) this.ports.failed(source.connectionId, source.leaseId);
    }
''')

# Stale transport failure must never disconnect a newer/different lease.
path = "server/rustEngine/externalRouting.test.ts"
needle = '''  it('bounds unresolved routes and never converts an explicit invalid token to a fresh join', () => {
'''
test = '''  it('disconnects only the exact current lease after asynchronous observation failure', () => {
    const { routing, commands } = fixture();
    routing.join(1, { type: 'join', mode: 'player', name: 'player' }, 'ui');
    routing.event({ kind: 'commandRejected', commandSequence: '0000000000000001',
      rejectionCode: 'InvalidCommand',
      rejectionDetail: 'InvalidCommand: no reserved legacy identity match' });
    routing.event({ kind: 'controllerJoinAssignment', controllerJoinAssignment: {
      requestSequence: '0000000000000002', connectionId: '0000000000000001',
      leaseId: '0000000000000009', snakeId: 12, controllerKind: 'player',
      resumeToken: 'a'.repeat(32), completedStep: '0000000000000000'
    } });
    routing.event({ kind: 'controllerJoinResolved', controllerJoinResolution: {
      requestSequence: '0000000000000002', matched: true, accepted: true
    } });

    const before = commands.length;
    routing.observationDeliveryFailed('0000000000000001', '0000000000000008');
    expect(commands).toHaveLength(before);

    routing.observationDeliveryFailed('0000000000000001', '0000000000000009');
    expect(commands.filter(command => command.kind === 'close')).toHaveLength(1);
    expect(commands.find(command => command.kind === 'close')).toMatchObject({
      value: { connectionId: '0000000000000001', leaseId: '0000000000000009' }
    });

    routing.observationDeliveryFailed('0000000000000001', '0000000000000009');
    expect(commands.filter(command => command.kind === 'close')).toHaveLength(1);
  });

  it('bounds unresolved routes and never converts an explicit invalid token to a fresh join', () => {
'''
text = read(path)
if needle not in text:
    raise RuntimeError("external routing test insertion point missing")
write(path, text.replace(needle, test, 1))

# Keep the mixed-consumer qualification harness on the same event contract used
# by the production server. Transactional replacement batches retain receipts.
path = "scripts/qualification/sensor-wall.ts"
replace_once(path,
'''import { ControllerDeliveryRouter, JoinDeliveryRouter, ReclaimDeliveryRouter } from '../../server/rustEngine/controllerDelivery.ts';
''',
'''import { ControllerDeliveryRouter, ControllerObservationRouter, JoinDeliveryRouter, ReclaimDeliveryRouter } from '../../server/rustEngine/controllerDelivery.ts';
''')
replace_once(path,
'''    const ordinary = new ControllerDeliveryRouter(Math.max(1, externalCount), {...ports,
      trySubmitReceipt: (sequence, receipt) => admission.trySubmitReceipt(sequence, receipt)});
''',
'''    const observations = new ControllerObservationRouter(Math.max(1, externalCount), {
      sensorCount: () => layout.inputSize, send: ports.send,
      failed: (connectionId, leaseId) => external.observationDeliveryFailed(connectionId, leaseId)});
    const ordinary = new ControllerDeliveryRouter(Math.max(1, externalCount), {...ports,
      trySubmitReceipt: (sequence, receipt) => admission.trySubmitReceipt(sequence, receipt)});
''')
replace_once(path,
'''          if (await generation.handle(event)) continue;
          if (event.controllerMessages) assert(ordinary.deliver(event.controllerMessages), 'overlapping observation delivery');
''',
'''          if (await generation.handle(event)) continue;
          if (event.controllerObservations) observations.deliver(event.controllerObservations);
          if (event.controllerMessages) assert(ordinary.deliver(event.controllerMessages), 'overlapping transactional controller delivery');
''')

# Update the real isolated Stage 3/6 integration fixture. It previously encoded
# the old invariant that each ordinary observation held completedStep until a
# local-send receipt arrived.
path = "server/rustEngine/checkpointPersistence.native.test.ts"
text = read(path)
text = text.replace(
    "import { ControllerDeliveryRouter, ReclaimDeliveryRouter, JoinDeliveryRouter } from './controllerDelivery.ts';",
    "import { ControllerObservationRouter, ReclaimDeliveryRouter, JoinDeliveryRouter } from './controllerDelivery.ts';",
)
old_interface = '''  /** Ordinary reliable messages preserved across command-result waits. */
  controllerMessages?: RustBackgroundControllerMessage[];
  /** Exact ordinary-step receipt completion. */
  controllerReceiptResolution?: RustControllerReceiptResolution;
'''
new_interface = '''  /** Ordered ordinary observations emitted after their authoritative step publishes. */
  controllerObservations?: RustBackgroundControllerMessage[];
  /** Receipt-driven mixed/replacement controller output, if a lifecycle transition requires it. */
  controllerMessages?: RustBackgroundControllerMessage[];
  /** Exact transactional ordinary-step receipt completion. */
  controllerReceiptResolution?: RustControllerReceiptResolution;
'''
if text.count(old_interface) != 1:
    raise RuntimeError("checkpoint event interface insertion point missing")
text = text.replace(old_interface, new_interface, 1)

start_marker = "        const ordinary = await waitForBackgroundEvent(session, event => event.kind === 'controllerMessages');\n"
end_marker = "        const timedHealth = await waitForBackgroundHealth(\n"
start = text.find(start_marker)
if start < 0:
    raise RuntimeError("checkpoint ordinary observation block start missing")
end = text.find(end_marker, start)
if end < 0:
    raise RuntimeError("checkpoint ordinary observation block end missing")
replacement = '''        const ordinary = await waitForBackgroundEvent(session, event => event.kind === 'controllerObservations');
        const messages = ordinary.controllerObservations;
        expect(messages).toHaveLength(1);
        const observation = messages![0]!;
        expect(observation).toMatchObject({ kind: 'observation', snakeId: Number(BigInt(`0x${assignment.frameV1Id}`)) });
        expect(observation.sensors).toHaveLength(83);
        expect(observation.sensors!.every(Number.isFinite)).toBe(true);
        expect(observation.sourceCompletedStep).toBe('0000000000000001');
        expect(BigInt(`0x${session.health().completedStep}`)).toBeGreaterThanOrEqual(1n);
        expect(session.health().loopState).not.toBe('externalDeliveryPending');
        expect(session.health().pendingExternalDeliveries).toBe('0000000000000000');
        const receipt: RustGenerationAssignmentReceipt = {
          operationEpoch: observation.operationEpoch, eventSequence: observation.eventSequence,
          connectionId: observation.connectionId, leaseId: observation.leaseId, accepted: true
        };
        // Generation and old ordinary-receipt commands cannot consume an already-published observation.
        session.submitGenerationAssignmentReceipt('000000000000000d', receipt);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '000000000000000d'))
          .resolves.toMatchObject({ kind: 'commandRejected' });
        session.submitControllerDeliveryReceipt('000000000000000e', receipt);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === '000000000000000e'))
          .resolves.toMatchObject({ kind: 'commandRejected' });

        const sent: unknown[] = [];
        const observationRouter = new ControllerObservationRouter(2, {
          send(connectionId, message) { sent.push({ connectionId, message }); return true; },
          failed() { throw new Error('accepted integration observation unexpectedly failed transport'); }
        });
        observationRouter.deliver(messages!);
        expect(sent).toEqual([{ connectionId: observation.connectionId, message: {
          type: 'sensors', tick: 1, snakeId: observation.snakeId, sensors: observation.sensors,
          meta: { x: observation.x, y: observation.y, dir: observation.direction }
        } }]);

        let admitReceipt = false;
        const admission = new BackgroundCommandAdmission({
          submitGenerationAssignmentReceipt: (sequence, completion) => session.submitGenerationAssignmentReceipt(sequence, completion),
          submitControllerJoinReceipt(sequence, completion) {
            if (!admitReceipt) throw new Error('QueueCountLimit: injected join receipt backpressure');
            session.submitControllerJoinReceipt(sequence, completion);
          },
          submitControllerReclaimReceipt(sequence, completion) {
            if (!admitReceipt) throw new Error('QueueCountLimit: injected reclaim receipt backpressure');
            session.submitControllerReclaimReceipt(sequence, completion);
          },
          submitControllerDeliveryReceipt(sequence, completion) {
            session.submitControllerDeliveryReceipt(sequence, completion);
          }
        }, 15n);
        const action: RustBackgroundControllerAction = {
          leaseId: observation.leaseId, connectionId: observation.connectionId,
          turn: 0.75, boost: false, clientTick: '000000000000002a'
        };
        expect(() => session.submitControllerAction('000000000000000f', { ...action, turn: NaN })).toThrow();
        let actionSequence = '';
        expect(admission.trySubmitControl(sequence => {
          actionSequence = sequence;
          session.submitControllerAction(sequence, action);
        })).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === actionSequence))
          .resolves.toMatchObject({ kind: 'controllerActionApplied' });
'''
text = text[:start] + replacement + text[end:]

# All later routine observations in this fixture use the post-publication event.
text = text.replace("event => event.kind === 'controllerMessages'", "event => event.kind === 'controllerObservations'")
text = text.replace(".controllerMessages!", ".controllerObservations!")
text = text.replace(".controllerMessages;", ".controllerObservations;")

# The next observation no longer needs a synthetic receipt before disconnect can apply.
start_marker = "        const nextOrdinary = await waitForBackgroundEvent(session, event => event.kind === 'controllerObservations');\n"
end_marker = "        const reclaimSequence = admission.nextSequence();\n"
start = text.find(start_marker)
if start < 0:
    raise RuntimeError("checkpoint next observation block start missing")
end = text.find(end_marker, start)
if end < 0:
    raise RuntimeError("checkpoint reclaim block anchor missing")
replacement = '''        const nextOrdinary = await waitForBackgroundEvent(session, event => event.kind === 'controllerObservations');
        const nextObservation = nextOrdinary.controllerObservations![0]!;
        expect(BigInt(`0x${nextObservation.sourceCompletedStep}`)).toBeGreaterThanOrEqual(2n);
        const close = { leaseId: nextObservation.leaseId, connectionId: nextObservation.connectionId };
        let disconnectSequence = '';
        expect(admission.trySubmitControl(sequence => {
          disconnectSequence = sequence;
          session.submitControllerDisconnect(sequence, close);
        })).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === disconnectSequence))
          .resolves.toMatchObject({ kind: 'controllerDisconnected', controllerDisconnect: {
            leaseId: close.leaseId, applied: true
          } });
        let duplicateDisconnectSequence = '';
        expect(admission.trySubmitControl(sequence => {
          duplicateDisconnectSequence = sequence;
          session.submitControllerDisconnect(sequence, close);
        })).toBe(true);
        await expect(waitForBackgroundEvent(session, event => event.commandSequence === duplicateDisconnectSequence))
          .resolves.toMatchObject({ kind: 'controllerDisconnected', controllerDisconnect: { applied: false } });
        await waitForBackgroundHealth(session, health =>
          BigInt(`0x${health.completedStep}`) >= BigInt(`0x${nextObservation.sourceCompletedStep}`));

'''
text = text[:start] + replacement + text[end:]

text = text.replace(
    "        expect(router.deliver([reclaimedObservation])).toBe(true);\n",
    "        observationRouter.deliver([reclaimedObservation]);\n",
)
text = text.replace(
    "        expect(router.deliver(beforeJoin)).toBe(true);\n",
    "        observationRouter.deliver(beforeJoin);\n",
)
text = text.replace(
    "        expect(session.health().completedStep).toBe(reclaimedObservation.sourceCompletedStep);\n",
    "        expect(BigInt(`0x${session.health().completedStep}`)).toBeGreaterThanOrEqual(BigInt(`0x${reclaimedObservation.sourceCompletedStep}`));\n",
)
old_joined = '''        const joinedRouter = new ControllerDeliveryRouter(2, {
          send() { return true; }, nextSequence() { return admission.nextSequence(); },
          trySubmitReceipt(sequence, receipt) { return admission.trySubmitReceipt(sequence, receipt); }
        });
        expect(joinedRouter.deliver(freshMessages)).toBe(true);
'''
new_joined = '''        const joinedRouter = new ControllerObservationRouter(2, {
          send() { return true; },
          failed() { throw new Error('joined observation unexpectedly failed transport'); }
        });
        joinedRouter.deliver(freshMessages);
'''
if text.count(old_joined) != 1:
    raise RuntimeError("checkpoint joined observation router block missing")
text = text.replace(old_joined, new_joined, 1)
write(path, text)

# Source comments and durable developer contracts must describe admission to the
# bounded Rust output stream, not a per-tick Node socket acknowledgement.
replace_optional(
    "native/src/engine/running_step.rs",
    '''//! External observations are exposed only after a complete physical step has
//! staged successfully, then the final swap waits for matching local Node send
//! results. Connected controlled-snake deaths stage fresh collision-safe
//! replacements and wait for the same reliable result boundary.
''',
    '''//! External observations are exposed only after a complete physical step has
//! staged successfully. Observation-only batches publish once their complete
//! owned output is admitted to the bounded Rust reliable queue; connected
//! controlled-snake deaths still stage fresh collision-safe replacements and
//! wait for matching local Node send results.
''',
)
replace_optional(
    "native/src/engine/running_step.rs",
    '''/// Borrowed bridge batch retained until every exact local send result resolves.
''',
    '''/// Borrowed staged external batch retained through output admission or transactional delivery.
''',
)
replace_optional(
    "native/src/engine/running_step.rs",
    '''    /// External observations become visible only after every physics substep and
    /// the nonterminal guard succeed. Their score markers and the complete world
    /// then publish together after matching local Node send acceptance.
''',
    '''    /// External observations become visible only after every physics substep and
    /// the nonterminal guard succeed. Observation-only score markers and the complete
    /// world then publish after bounded Rust output admission; replacement assignments
    /// retain exact local Node send acceptance before publication.
''',
)
replace_optional(
    "native/src/engine/mod.rs",
    "/// Bounded reliable ordinary-step observations and exact local-send receipts.\n",
    "/// Bounded external-controller output and transactional local-send receipts.\n",
)
replace_optional(
    "native/src/engine/sensors.rs",
    '''    /// Commit the sampled score boundary after real internal consumption or
    /// successful Node acceptance for the matching external assignment.
''',
    '''    /// Commit the sampled score boundary after real internal consumption or
    /// bounded authoritative-output admission for the matching external observation.
''',
)
replace_optional(
    "native/src/engine/world_step.rs",
    "    /// Packed external observations still awaiting matching Node acceptance.\n",
    "    /// Packed external observations retained until authoritative output or transactional delivery resolves.\n",
)
replace_optional(
    "native/src/engine/world_step.rs",
    "    /// Borrow publication buffers after all checks were completed before Node delivery.\n",
    "    /// Borrow publication buffers after all external-output checks were completed.\n",
)
replace_optional(
    "native/src/engine/inspector.rs",
    "    /// Join only the exact successful step and, for external rows, its accepted socket delivery.\n",
    "    /// Join only the exact successful step and, for external rows, its accepted authoritative observation boundary.\n",
)
replace_optional(
    "server/rustEngine/backgroundOutput.ts",
    "/** One bounded output consumer shared by generation and ordinary delivery barriers. */\n",
    "/** One bounded output consumer for asynchronous observations and transactional lifecycle barriers. */\n",
)
replace_optional(
    "src/protocol/rustBackground.ts",
    "  /** Outstanding local controller sends. */\n  pendingExternalDeliveries: RustBackgroundIdentity;\n",
    "  /** Outstanding receipt-driven controller assignment sends; ordinary observations are asynchronous. */\n  pendingExternalDeliveries: RustBackgroundIdentity;\n",
)
replace_optional(
    "src/protocol/rustBackground.ts",
    "  /** Rust-delivered sensor-v3 vector for observations. */\n  sensors?: number[];\n",
    "  /** Rust-delivered canonical sensor vector for observations. */\n  sensors?: number[];\n",
)

# Keep current API documentation aligned with the real v4 default beside the
# new asynchronous observation semantics.
path = "docs/API-instructions.md"
replace_optional(path,
'''The ordinary production default is the frozen `v3` layout, described below.
Explicitly admitted candidate v4 profiles use `24 + 6 * bubbleBins` inputs for
Basic and `24 + 15 * bubbleBins` for Motion, with 8..32 bins. They require the
exact Rust canonical sensor contract carried in welcome metadata; equal widths
do not imply equal meanings. Their workload/ablation and production-default
qualification remains open.

The frozen `v3` size is:
''',
'''Fresh current runs use the Rust v4 Motion/B16 sensor contract by default.
V4 Basic uses `24 + 6 * bubbleBins` inputs and v4 Motion uses
`24 + 15 * bubbleBins`, with 8..32 bins; Motion/B16 therefore has 264 inputs.
Clients must use the exact canonical contract in `welcome.sensorSpec`, because
equal widths do not imply equal meanings. The frozen v3 layout remains a
compatibility contract for retained runs that explicitly carry it.

The frozen `v3` size is:
''')

# Generated logs from the one-shot implementation workflow are CI artifacts,
# not source files.
for generated in ("patch.log", "verify.log"):
    try:
        Path(generated).unlink()
    except FileNotFoundError:
        pass

print("final async observation regressions, qualification harness and documentation applied")
