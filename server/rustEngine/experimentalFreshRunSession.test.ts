import { describe, expect, it } from 'vitest';
import type { ManagedCheckpointCommitResult } from './checkpointPersistenceClient.ts';
import type {
  ManagedCheckpointDescriptor,
  U64Hex
} from './checkpointPersistenceProtocol.ts';
import {
  createExperimentalFreshRunSession,
  validateExperimentalFreshRunBinding,
  type ExperimentalFreshRunFrameV1,
  type ExperimentalFreshRunNativeHandle,
  type ExperimentalFreshRunSnapshot
} from './experimentalFreshRunSession.ts';
import type { NativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Deterministic fake source identity used by the production-build handshake. */
const SOURCE_IDENTITY: NativeSourceIdentity = {
  sha256: 'a'.repeat(64),
  fileCount: 1,
  totalCanonicalBytes: 1,
  totalAccountedPathBytes: 1,
  manifest: []
};

/** Exact run-start descriptor returned by the fake Rust authority. */
function descriptor(operationId = '01010101010101010101010101010101'): ManagedCheckpointDescriptor {
  const logicalRootSha256 = 'b'.repeat(64);
  return {
    protocolVersion: 1,
    operationId,
    transitionEpoch: '0000000000000007',
    runId: 'fake-lineage',
    generation: '0000000000000001',
    completedStep: '0000000000000000',
    boundaryKind: 'run-start',
    checkpointFormatVersion: '0000000000000003',
    stateVersion: '0000000000000001',
    graphLayoutVersion: '0000000000000001',
    managedRoot: 'checkpoint-v3',
    relativeFilename: `${logicalRootSha256}.checkpoint-v3`,
    logicalRootSha256,
    storedByteCount: '0000000000001000',
    decodedByteCount: '0000000000002000',
    roleCount: '0000000000000005',
    populationCount: '0000000000000037',
    weightCount: '00000000000b4bb6',
    recurrentStateCount: '0000000000000370',
    weightsEncoding: 'raw-f32le-v1',
    recurrentStateEncoding: 'raw-f32le-v1',
    graphLayoutSha256: 'c'.repeat(64),
    writeValidationPolicy: 'write-hash-count-fsync-rename-v1'
  };
}

/** Convert one descriptor to the exact successful worker result. */
function commitResult(value: ManagedCheckpointDescriptor): ManagedCheckpointCommitResult {
  return {
    operationId: value.operationId,
    transitionEpoch: value.transitionEpoch,
    runId: value.runId,
    checkpointId: value.logicalRootSha256,
    descriptor: value
  };
}

/** Build one complete scalar snapshot with no population-sized fields. */
function snapshot(
  phase: ExperimentalFreshRunSnapshot['phase'],
  overrides: Partial<ExperimentalFreshRunSnapshot> = {}
): ExperimentalFreshRunSnapshot {
  return {
    phase,
    transitionEpoch: '0000000000000007',
    generation: '0000000000000001',
    completedStep: '0000000000000000',
    checkpointPublished: false,
    persistenceAcknowledged: false,
    authorityPublished: false,
    initialFramePublished: false,
    snakeCount: '0000000000000000',
    pelletCount: '0000000000000000',
    faultDetail: undefined,
    ...overrides
  };
}

/** Build one minimal complete frame-v1 payload with exact routing metadata. */
function initialFrameV1(): ExperimentalFreshRunFrameV1 {
  const floats = new Float32Array([1, 0, 0, 3_500, 0, 0, 1, 0]);
  return {
    bytes: new Uint8Array(floats.buffer, floats.byteOffset, floats.byteLength),
    generation: '0000000000000001',
    totalSnakes: '0000000000000000',
    aliveSnakes: '0000000000000000',
    pellets: '0000000000000000',
    floatLength: '0000000000000008',
    byteLength: '0000000000000020'
  };
}

/** Captured exact native constructor inputs. */
interface CapturedInputs {
  /** Bounded opaque run identity. */
  runId: string;
  /** Canonical Uint32 seed. */
  seedHex: string;
  /** Canonical positive u64 memory ceiling. */
  memoryCeilingBytesHex: U64Hex;
}

/** Mutable fake-native evidence retained across one test. */
interface FakeEvidence {
  /** Constructor inputs crossing N-API. */
  constructed: CapturedInputs[];
  /** Exact publication options received from the handoff. */
  published: unknown[];
  /** Exact descriptors acknowledged back into Rust. */
  acknowledged: ManagedCheckpointDescriptor[];
}

/** Minimal fake native owner with the exact production class surface. */
class FakeFreshRunSession implements ExperimentalFreshRunNativeHandle {
  /** Current bounded scalar state. */
  private current = snapshot('pendingDurability');
  /** Shared test evidence sink. */
  private readonly evidence: FakeEvidence;

  /** Capture only exact scalar constructor inputs. */
  public constructor(evidence: FakeEvidence, runId: string, seedHex: string, memoryHex: U64Hex) {
    this.evidence = evidence;
    evidence.constructed.push({ runId, seedHex, memoryCeilingBytesHex: memoryHex });
  }

  /** Return one already-constructed fake boundary. */
  public async initialize(): Promise<unknown> {
    return this.current;
  }

  /** Return Rust's exact descriptor for the supplied operation token. */
  public async publishRunStartCheckpoint(options: {
    managedDirectory: string;
    operationId: string;
  }): Promise<unknown> {
    this.evidence.published.push(options);
    this.current = snapshot('awaitingPersistence', { checkpointPublished: true });
    return descriptor(options.operationId);
  }

  /** Retain the exact committed descriptor. */
  public acknowledgeRunStartPersistence(value: ManagedCheckpointDescriptor): void {
    this.evidence.acknowledged.push(value);
    this.current = snapshot('durableBoundary', {
      checkpointPublished: true,
      persistenceAcknowledged: true
    });
  }

  /** Return the one successful scalar activation. */
  public async activateRunningAuthority(): Promise<unknown> {
    this.current = snapshot('running', {
      checkpointPublished: true,
      persistenceAcknowledged: true,
      authorityPublished: true,
      snakeCount: '0000000000000041',
      pelletCount: '0000000000000dac'
    });
    return {
      worldEpoch: '0000000000000007',
      generation: '0000000000000001',
      completedStep: '0000000000000000',
      populationEpoch: '0000000000000001'
    };
  }

  /** Return one Rust-shaped initial display frame and retain its scalar marker. */
  public async publishInitialFrameV1(): Promise<unknown> {
    this.current = snapshot('running', {
      checkpointPublished: true,
      persistenceAcknowledged: true,
      authorityPublished: true,
      initialFramePublished: true,
      snakeCount: '0000000000000041',
      pelletCount: '0000000000000dac'
    });
    return initialFrameV1();
  }

  /** Read the current scalar state. */
  public snapshot(): unknown {
    return this.current;
  }
}

/** Build a production-shaped fake addon around one evidence sink. */
function fakeBinding(evidence: FakeEvidence): unknown {
  return {
    nativeAddonSourceSha256: () => SOURCE_IDENTITY.sha256,
    nativeAddonBuildTarget: () => 'x86_64-pc-windows-msvc',
    nativeAddonBuildProfile: () => 'release',
    nativeAddonBuildClass: () => 'production',
    nativeAddonRustcVersion: () => 'rustc 1.90.0',
    nativeAddonBuildContractSha256: () => `sha256:${'d'.repeat(64)}`,
    experimentalEngineContractVersion: () => 1,
    ExperimentalRustEngine: class {},
    ExperimentalStage6aFreshRunSession: class extends FakeFreshRunSession {
      /** Bind the test evidence without changing the native constructor shape. */
      public constructor(runId: string, seedHex: string, memoryHex: U64Hex) {
        super(evidence, runId, seedHex, memoryHex);
      }
    }
  };
}

/** Create a fresh empty evidence sink. */
function createEvidence(): FakeEvidence {
  return { constructed: [], published: [], acknowledged: [] };
}

describe('experimental fixed-P0 fresh-run session', () => {
  it('sends only exact identity inputs and composes Rust descriptor authority unchanged', async () => {
    const evidence = createEvidence();
    const committed: ManagedCheckpointDescriptor[] = [];
    const session = createExperimentalFreshRunSession({
      binding: fakeBinding(evidence),
      sourceIdentity: SOURCE_IDENTITY,
      runId: 'lineage-🐍',
      seed: 0x89ab_cdef,
      memoryCeilingBytes: 4n * 1024n * 1024n * 1024n,
      managedDirectory: 'C:\\controlled\\checkpoint-v3',
      persistence: {
        async commit(value) {
          const parsed = value as ManagedCheckpointDescriptor;
          committed.push(parsed);
          return commitResult(parsed);
        }
      }
    });

    expect(evidence.constructed).toEqual([{
      runId: 'lineage-🐍',
      seedHex: '89abcdef',
      memoryCeilingBytesHex: '0000000100000000'
    }]);
    await expect(session.initialize()).resolves.toMatchObject({ phase: 'pendingDurability' });
    const result = await session.commitPendingRunStart('01010101010101010101010101010101');
    expect(evidence.published).toEqual([{
      managedDirectory: 'C:\\controlled\\checkpoint-v3',
      operationId: '01010101010101010101010101010101'
    }]);
    expect(committed).toEqual([descriptor()]);
    expect(evidence.acknowledged).toEqual([descriptor()]);
    expect(result).toEqual(commitResult(descriptor()));
    await expect(session.activateRunningAuthority()).resolves.toEqual({
      worldEpoch: '0000000000000007',
      generation: '0000000000000001',
      completedStep: '0000000000000000',
      populationEpoch: '0000000000000001'
    });
    expect(session.snapshot()).toMatchObject({
      phase: 'running',
      snakeCount: '0000000000000041',
      pelletCount: '0000000000000dac'
    });
    await expect(session.publishInitialFrameV1()).resolves.toMatchObject({
      generation: '0000000000000001',
      floatLength: '0000000000000008',
      byteLength: '0000000000000020'
    });
    expect(session.snapshot()).toMatchObject({
      phase: 'running',
      initialFramePublished: true
    });
  });

  it('rejects narrowed or malformed constructor inputs before native construction', () => {
    const evidence = createEvidence();
    const base = {
      binding: fakeBinding(evidence),
      sourceIdentity: SOURCE_IDENTITY,
      runId: 'lineage',
      seed: 1,
      memoryCeilingBytes: 1n,
      managedDirectory: 'managed',
      persistence: { async commit(value: unknown) { return commitResult(value as ManagedCheckpointDescriptor); } }
    };
    expect(() => createExperimentalFreshRunSession({ ...base, seed: -1 })).toThrow(/unsigned 32/i);
    expect(() => createExperimentalFreshRunSession({ ...base, seed: 2 ** 32 })).toThrow(/unsigned 32/i);
    expect(() => createExperimentalFreshRunSession({
      ...base,
      memoryCeilingBytes: 1 as unknown as bigint
    })).toThrow(/bigint/i);
    expect(() => createExperimentalFreshRunSession({ ...base, memoryCeilingBytes: 0n })).toThrow(
      /positive unsigned 64/i
    );
    expect(() => createExperimentalFreshRunSession({ ...base, runId: '\ud800' })).toThrow(
      /well-formed UTF-16/i
    );
    expect(() => createExperimentalFreshRunSession({ ...base, runId: 'x'.repeat(257) })).toThrow(
      /256 UTF-8 bytes/i
    );
    expect(() => createExperimentalFreshRunSession({
      ...base,
      managedDirectory: '\ud800'
    })).toThrow(/managedDirectory.*well-formed UTF-16/i);
    expect(() => createExperimentalFreshRunSession({
      ...base,
      managedDirectory: 'x'.repeat(32_769)
    })).toThrow(/managedDirectory.*32768 UTF-8 bytes/i);
    expect(evidence.constructed).toEqual([]);
  });

  it('rejects a stale addon or expanded fine-grained class surface', () => {
    const evidence = createEvidence();
    const staleIdentity = { ...SOURCE_IDENTITY, sha256: 'e'.repeat(64) };
    expect(() => validateExperimentalFreshRunBinding(fakeBinding(evidence), staleIdentity)).toThrow(
      /stale/i
    );

    class ExpandedFreshRunSession extends FakeFreshRunSession {
      /** Prohibited per-snake mutation must never enter this class surface. */
      public snakeStep(): void {}
    }
    const expanded = {
      ...(fakeBinding(evidence) as Record<string, unknown>),
      ExperimentalStage6aFreshRunSession: ExpandedFreshRunSession
    };
    expect(() => validateExperimentalFreshRunBinding(expanded, SOURCE_IDENTITY)).toThrow(
      /unsupported surface|prohibited/i
    );
  });

  it('rejects numeric or internally partial native scalar output', async () => {
    const evidence = createEvidence();
    const binding = fakeBinding(evidence) as Record<string, unknown>;
    class InvalidOutputFreshRunSession extends FakeFreshRunSession {
      /** Return a deliberately narrowed transition token. */
      public override async initialize(): Promise<unknown> {
        return { ...snapshot('pendingDurability'), transitionEpoch: 7 };
      }
    }
    binding['ExperimentalStage6aFreshRunSession'] = class extends InvalidOutputFreshRunSession {
      /** Bind evidence for the invalid-output fixture. */
      public constructor(runId: string, seedHex: string, memoryHex: U64Hex) {
        super(evidence, runId, seedHex, memoryHex);
      }
    };
    const session = createExperimentalFreshRunSession({
      binding,
      sourceIdentity: SOURCE_IDENTITY,
      runId: 'lineage',
      seed: 1,
      memoryCeilingBytes: 1n,
      managedDirectory: 'managed',
      persistence: { async commit(value) { return commitResult(value as ManagedCheckpointDescriptor); } }
    });
    await expect(session.initialize()).rejects.toThrow(/transitionEpoch.*hexadecimal/i);
  });

  it('rejects narrowed, expanded, or length-inconsistent native frame output', async () => {
    const evidence = createEvidence();
    const binding = fakeBinding(evidence) as Record<string, unknown>;
    let output: unknown = { ...initialFrameV1(), totalSnakes: 0 };
    class InvalidFrameFreshRunSession extends FakeFreshRunSession {
      /** Return the selected malformed frame through the normal wrapper parser. */
      public override async publishInitialFrameV1(): Promise<unknown> {
        return output;
      }
    }
    binding['ExperimentalStage6aFreshRunSession'] = class extends InvalidFrameFreshRunSession {
      /** Bind evidence for the malformed-frame fixture. */
      public constructor(runId: string, seedHex: string, memoryHex: U64Hex) {
        super(evidence, runId, seedHex, memoryHex);
      }
    };
    const session = createExperimentalFreshRunSession({
      binding,
      sourceIdentity: SOURCE_IDENTITY,
      runId: 'lineage',
      seed: 1,
      memoryCeilingBytes: 1n,
      managedDirectory: 'managed',
      persistence: { async commit(value) { return commitResult(value as ManagedCheckpointDescriptor); } }
    });
    await expect(session.publishInitialFrameV1()).rejects.toThrow(/totalSnakes.*hexadecimal/i);
    output = { ...initialFrameV1(), unexpectedPopulation: [] };
    await expect(session.publishInitialFrameV1()).rejects.toThrow(/unknown field/i);
    output = { ...initialFrameV1(), byteLength: '000000000000001c' };
    await expect(session.publishInitialFrameV1()).rejects.toThrow(/lengths disagree/i);
    output = { ...initialFrameV1(), bytes: new Uint8Array(28) };
    await expect(session.publishInitialFrameV1()).rejects.toThrow(/payload length disagrees/i);
  });

  it('accepts only a bounded internally consistent terminal fault snapshot', () => {
    const evidence = createEvidence();
    const binding = fakeBinding(evidence) as Record<string, unknown>;
    class FaultSnapshotFreshRunSession extends FakeFreshRunSession {
      /** Return the supplied test fault shape through the normal wrapper parser. */
      public override snapshot(): unknown {
        return { phase: 'faulted', faultDetail: 'construction panic' };
      }
    }
    binding['ExperimentalStage6aFreshRunSession'] = class extends FaultSnapshotFreshRunSession {
      /** Bind evidence for the fault-output fixture. */
      public constructor(runId: string, seedHex: string, memoryHex: U64Hex) {
        super(evidence, runId, seedHex, memoryHex);
      }
    };
    const session = createExperimentalFreshRunSession({
      binding,
      sourceIdentity: SOURCE_IDENTITY,
      runId: 'lineage',
      seed: 1,
      memoryCeilingBytes: 1n,
      managedDirectory: 'managed',
      persistence: { async commit(value) { return commitResult(value as ManagedCheckpointDescriptor); } }
    });
    expect(session.snapshot()).toEqual({
      phase: 'faulted',
      transitionEpoch: undefined,
      generation: undefined,
      completedStep: undefined,
      checkpointPublished: undefined,
      persistenceAcknowledged: undefined,
      authorityPublished: undefined,
      initialFramePublished: undefined,
      snakeCount: undefined,
      pelletCount: undefined,
      faultDetail: 'construction panic'
    });

    FaultSnapshotFreshRunSession.prototype.snapshot = () => ({
      phase: 'faulted',
      faultDetail: 'x'.repeat(513)
    });
    expect(() => session.snapshot()).toThrow(/faultDetail.*512 UTF-8 bytes/i);
  });
});
