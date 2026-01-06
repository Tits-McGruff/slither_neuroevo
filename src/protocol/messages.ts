import type { CoreSettings, SettingsUpdate } from './settings';
import type { GraphSpec } from '../brains/graph/schema.ts';

/** Observer/camera behavior tuning for the simulation. */
export interface ObserverSettings {
  focusRecheckSeconds: number;
  focusSwitchMargin: number;
  earlyEndMinSeconds: number;
  earlyEndAliveThreshold: number;
  defaultViewMode: 'overview' | 'follow';
  overviewPadding: number;
  snapZoomOutInOverview: boolean;
  zoomLerpFollow: number;
  zoomLerpOverview: number;
  overviewExtraWorldMargin: number;
}

/** Collision system configuration for physics substeps and spatial hashing. */
export interface CollisionSettings {
  substepMaxDt: number;
  skipSegments: number;
  hitScale: number;
  cellSize: number;
  neighborRange: number;
}

/** Initialization settings accepted by the worker reset flow. */
export interface InitSettings extends Partial<CoreSettings> {
  worldRadius?: number;
  observer?: Partial<ObserverSettings>;
  collision?: Partial<CollisionSettings>;
}

/** Serialized genome representation for export/import. */
export interface GenomeJSON {
  archKey: string;
  brainType?: string;
  weights: number[];
  fitness?: number;
}

/** Population export payload. */
export interface PopulationExport {
  generation: number;
  archKey: string;
  genomes: GenomeJSON[];
}

/** Population import payload, optionally including HoF. */
export interface PopulationImportData {
  generation?: number;
  archKey?: string;
  genomes?: GenomeJSON[];
  hof?: HallOfFameEntry[];
}

/** Fitness summary for a single generation. */
export interface FitnessData {
  gen: number;
  avgFitness: number;
  maxFitness: number;
  minFitness: number;
}

/** Historical fitness metrics used by charts and UI. */
export interface FitnessHistoryEntry {
  gen: number;
  best: number;
  avg: number;
  min: number;
  speciesCount?: number;
  topSpeciesSize?: number;
  avgWeight?: number;
  weightVariance?: number;
}

/** Brain visualizer layer payload. */
export interface VizLayer {
  count: number;
  activations: ArrayLike<number> | null;
  isRecurrent?: boolean;
}

/** Brain visualizer payload for the UI. */
export interface VizData {
  kind: string;
  layers: VizLayer[];
}

/** Hall of Fame entry for resurrecting elite snakes. */
export interface HallOfFameEntry {
  gen: number;
  seed: number;
  fitness: number;
  points: number;
  length: number;
  genome: GenomeJSON;
}

/** Stats emitted alongside frame buffers. */
export interface FrameStats {
  gen: number;
  alive: number;
  fps: number;
  fitnessData?: FitnessData;
  fitnessHistory?: FitnessHistoryEntry[];
  viz?: VizData;
  hofEntry?: HallOfFameEntry;
}

/** Worker init message from the main thread. */
export type InitMessage = {
  type: 'init';
  settings?: InitSettings;
  updates?: SettingsUpdate[];
  resetCfg?: boolean;
  viewW?: number;
  viewH?: number;
  population?: GenomeJSON[];
  generation?: number;
  graphSpec?: GraphSpec | null;
  stackOrder?: string[];
};

/** Worker settings update message from the main thread. */
export type UpdateSettingsMessage = {
  type: 'updateSettings';
  updates?: SettingsUpdate[];
};

/** Main-thread action message for view and sim speed changes. */
export type ActionMessage =
  | {
      type: 'action';
      action: 'toggleView';
    }
  | {
      type: 'action';
      action: 'simSpeed';
      value: number;
    };

/** Resize notification to update viewport dimensions. */
export type ResizeMessage = {
  type: 'resize';
  viewW: number;
  viewH: number;
};

/** Toggle visualization streaming from worker/server. */
export type VizMessage = {
  type: 'viz';
  enabled: boolean;
};

/** Request to resurrect a saved genome. */
export type ResurrectMessage = {
  type: 'resurrect';
  genome: GenomeJSON;
};

/** Import a population payload into the worker. */
export type ImportMessage = {
  type: 'import';
  data: PopulationImportData;
};

/** Export request for population payload. */
export type ExportMessage = {
  type: 'export';
};

/** God mode actions for kill/move operations. */
export type GodModeMessage =
  | {
      type: 'godMode';
      action: 'kill';
      snakeId: number;
    }
  | {
      type: 'godMode';
      action: 'move';
      snakeId: number;
      x: number;
      y: number;
    };

/** Union of messages from main thread to worker. */
export type MainToWorkerMessage =
  | InitMessage
  | UpdateSettingsMessage
  | ActionMessage
  | ResizeMessage
  | VizMessage
  | ResurrectMessage
  | ImportMessage
  | ExportMessage
  | GodModeMessage;

/** Worker-to-main frame payload containing the render buffer. */
export type FrameMessage = {
  type: 'frame';
  buffer: ArrayBuffer;
  stats: FrameStats;
};

/** Worker-to-main export result payload. */
export type ExportResultMessage = {
  type: 'exportResult';
  data: PopulationExport;
};

/** Worker-to-main import result payload. */
export type ImportResultMessage = {
  type: 'importResult';
  ok: boolean;
  reason: string | null;
  generation: number;
  used: number;
  total: number;
};

/** Union of messages from worker to main thread. */
export type WorkerToMainMessage = FrameMessage | ExportResultMessage | ImportResultMessage;
