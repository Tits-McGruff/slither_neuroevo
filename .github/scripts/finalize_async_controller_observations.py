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


# Exact output-size proof must cover the full ReliableEvent before authority
# publication, not just the inner RunningAuthorityEvent payload.
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

# Native regression for the exact invariant: complete owned observation output
# fits the reservation before the step can publish, and no Node receipt is
# needed to retire the scheduler ticket.
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

# Avoid manufacturing receipt objects on the hot asynchronous observation path.
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

# Exact stale-failure routing regression: an old observation failure must never
# close a newer or differently leased route.
path = "server/rustEngine/externalRouting.test.ts"
needle = '''  it('queues action/close once and never retries after accepted admission', () => {
'''
test = '''  it('disconnects only the exact current lease after asynchronous observation failure', () => {
    const { routing, commands } = fixture();
    routing.join(1, { type: 'join', mode: 'player', name: 'player' }, 'ui');
    routing.event({
      kind: 'commandRejected', commandSequence: '0000000000000001',
      rejectionCode: 'invalid_command', rejectionDetail: 'no reserved legacy identity match'
    });
    routing.event({
      kind: 'controllerJoinAssignment',
      controllerJoinAssignment: {
        requestSequence: '0000000000000002', controllerKind: 'player',
        connectionId: '0000000000000001', leaseId: '0000000000000009', snakeId: 12,
        completedStep: '0000000000000000', resumeToken: 'abcdefghijklmnopqrstuvwxyzABCDEF',
        identityKey: 'player:player'
      }
    });
    routing.event({
      kind: 'controllerJoinResolved',
      controllerJoinResolution: {
        requestSequence: '0000000000000002', matched: true, accepted: true
      }
    });

    const before = commands.length;
    routing.observationDeliveryFailed('0000000000000001', '0000000000000008');
    expect(commands).toHaveLength(before);

    routing.observationDeliveryFailed('0000000000000001', '0000000000000009');
    const closes = commands.filter(command => command.kind === 'close');
    expect(closes).toHaveLength(1);
    expect(closes[0]).toMatchObject({
      kind: 'close',
      value: {
        connectionId: '0000000000000001',
        leaseId: '0000000000000009'
      }
    });

    routing.observationDeliveryFailed('0000000000000001', '0000000000000009');
    expect(commands.filter(command => command.kind === 'close')).toHaveLength(1);
  });

  it('queues action/close once and never retries after accepted admission', () => {
'''
text = read(path)
if needle not in text:
    raise RuntimeError("external routing test insertion point missing")
write(path, text.replace(needle, test, 1))

# Source comments must match the new authoritative observation boundary.
replace_once(
    "native/src/engine/sensors.rs",
    "    /// Commit the sampled score boundary after real internal consumption or\n    /// successful Node acceptance for the matching external assignment.\n",
    "    /// Commit the sampled score boundary after real internal consumption or\n    /// bounded authoritative-output admission for the matching external observation.\n",
)
replace_once(
    "native/src/engine/world_step.rs",
    "    /// Packed external observations still awaiting matching Node acceptance.\n",
    "    /// Packed external observations retained until their authoritative output or transactional delivery resolves.\n",
)
replace_once(
    "native/src/engine/world_step.rs",
    "    /// Borrow publication buffers after all checks were completed before Node delivery.\n",
    "    /// Borrow publication buffers after all external-output checks were completed.\n",
)
replace_once(
    "native/src/engine/inspector.rs",
    "    /// Join only the exact successful step and, for external rows, its accepted socket delivery.\n",
    "    /// Join only the exact successful step and, for external rows, its accepted authoritative observation boundary.\n",
)
replace_once(
    "src/protocol/rustBackground.ts",
    "  /** Outstanding local controller sends. */\n  pendingExternalDeliveries: RustBackgroundIdentity;\n",
    "  /** Outstanding receipt-driven controller assignment sends; ordinary observations are asynchronous. */\n  pendingExternalDeliveries: RustBackgroundIdentity;\n",
)
replace_once(
    "src/protocol/rustBackground.ts",
    "  /** Rust-delivered sensor-v3 vector for observations. */\n  sensors?: number[];\n",
    "  /** Rust-delivered canonical sensor vector for observations. */\n  sensors?: number[];\n",
)

# Bring the Slither-local API sensor section in line with the current Rust v4
# default instead of leaving the old v3-default text beside the new async rules.
path = "docs/API-instructions.md"
replace_once(path,
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

# Generated verification logs from the first one-shot workflow are artifacts,
# not source-controlled project files.
for generated in ("patch.log", "verify.log"):
    try:
        Path(generated).unlink()
    except FileNotFoundError:
        pass

print("final async observation regressions and documentation applied")
