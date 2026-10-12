/** On-demand entity identity; the binary frame format remains unchanged. */

/** Run-scoped request for either a displayed physical entity or a population owner. */
export type SelectEntityMsg = {
  /** Message discriminator. */
  type: 'selectEntity';
  /** Client-owned correlation identity. */
  requestId: string;
  /** Process identity from the observed welcome. */
  expectedSessionId: string;
  /** Run identity from the observed welcome. */
  expectedRunId: string;
  /** Generation from the observed frame. */
  expectedGeneration: number;
} & ({ /** Exact frame-v1 ID. */ snakeId: number; populationSlot?: never }
  | { /** Persistent evolved-population owner. */ populationSlot: number; snakeId?: never });

/** Small Rust-authored physical snapshot; all opaque identities remain exact strings. */
export interface SelectedEntity {
  /** Canonical nonce/world/physical identity token for subsequent mutations. */
  token: string;
  /** Current frame-v1 ID. */
  snakeId: number;
  /** Persistent population owner when this is an evolved life. */
  populationSlot?: number | null;
  /** Exact positive ordinal within this generation. */
  lifeOrdinal?: string | null;
  /** Current head X. */
  x: number;
  /** Current head Y. */
  y: number;
  /** Current collision radius. */
  radius: number;
  /** Current display skin. */
  skin: number;
}

/** Reliable result, including honest dead/pending and stale-scope rejection. */
export interface EntitySelectedMsg {
  /** Message discriminator. */
  type: 'entitySelected';
  /** Original correlation identity. */
  requestId: string;
  /** Whether a current physical life was found. */
  found: boolean;
  /** Scalar selected entity when found. */
  entity?: SelectedEntity;
  /** Bounded admission or stale-scope explanation. */
  reason?: string;
}

/** Check fixed canonical token syntax without interpreting native ownership. */
export function isEntitySelectionToken(value: unknown): value is string {
  return typeof value === 'string' && /^[0-9a-f]{64}$/u.test(value)
    && value.slice(32, 48) !== '0000000000000000' && value.slice(48) !== '0000000000000000';
}
