import type { CoreSettings, SettingsUpdate } from './settings';

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

export interface CollisionSettings {
  substepMaxDt: number;
  skipSegments: number;
  hitScale: number;
  cellSize: number;
  neighborRange: number;
}

export interface InitSettings extends Partial<CoreSettings> {
  worldRadius?: number;
  observer?: Partial<ObserverSettings>;
  collision?: Partial<CollisionSettings>;
}

export interface GenomeJSON {
  archKey: string;
  weights: number[];
  fitness?: number;
}

export interface PopulationExport {
  generation: number;
  archKey: string;
  genomes: GenomeJSON[];
}

export interface PopulationImportData {
  generation?: number;
  archKey?: string;
  genomes?: GenomeJSON[];
  hof?: HallOfFameEntry[];
}

export interface FitnessData {
  gen: number;
  avgFitness: number;
  maxFitness: number;
  minFitness: number;
}

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

export interface VizMLPData {
  layerSizes: number[];
  _bufs: number[][];
}

export interface VizGRUData {
  hiddenSize: number;
  h: number[];
}

export interface VizHeadData {
  outSize: number;
}

export type VizData =
  | {
      kind: 'mlp';
      mlp: VizMLPData;
    }
  | {
      kind: string;
      mlp: VizMLPData;
      gru: VizGRUData | null;
      head: VizHeadData | null;
    };

export interface HallOfFameEntry {
  gen: number;
  seed: number;
  fitness: number;
  points: number;
  length: number;
  genome: GenomeJSON;
}

export interface FrameStats {
  gen: number;
  alive: number;
  fps: number;
  fitnessData?: FitnessData;
  fitnessHistory?: FitnessHistoryEntry[];
  viz?: VizData;
  hofEntry?: HallOfFameEntry;
}

export type InitMessage = {
  type: 'init';
  settings?: InitSettings;
  updates?: SettingsUpdate[];
  resetCfg?: boolean;
  viewW?: number;
  viewH?: number;
  population?: GenomeJSON[];
  generation?: number;
};

export type UpdateSettingsMessage = {
  type: 'updateSettings';
  updates?: SettingsUpdate[];
};

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

export type ResizeMessage = {
  type: 'resize';
  viewW: number;
  viewH: number;
};

export type VizMessage = {
  type: 'viz';
  enabled: boolean;
};

export type ResurrectMessage = {
  type: 'resurrect';
  genome: GenomeJSON;
};

export type ImportMessage = {
  type: 'import';
  data: PopulationImportData;
};

export type ExportMessage = {
  type: 'export';
};

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

export type FrameMessage = {
  type: 'frame';
  buffer: ArrayBuffer;
  stats: FrameStats;
};

export type ExportResultMessage = {
  type: 'exportResult';
  data: PopulationExport;
};

export type ImportResultMessage = {
  type: 'importResult';
  ok: boolean;
  reason: string | null;
  generation: number;
  used: number;
  total: number;
};

export type WorkerToMainMessage = FrameMessage | ExportResultMessage | ImportResultMessage;
