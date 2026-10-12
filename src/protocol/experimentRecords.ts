/** Rust-authored fixed metadata, separate from population/checkpoint payloads. */
export interface GenerationExperimentRecord {
  /** History schema; the paired HoF payload explicitly uses schema two. */
  version: 1;
  /** Exactly 512 little-endian bytes encoded as lowercase hex. */
  historyHex: string;
  /** Exactly 256 little-endian bytes encoded as lowercase hex. */
  hallOfFameHex: string;
}

/** Explicit assessment modes retained in the native fixed-record policy. */
export type PointAssessmentMethod = 'reset' | 'cumulative-net' | 'cumulative-gross' | 'best-life';
/** Actual fitness component source, including the physical legacy evaluator. */
export type FitnessBundleMethod = 'generation-totals' | 'follow-score' | 'legacy-terminal';
/** Native reductions projected as bounded metadata, without recalculating fitness. */
export interface ExperimentStatistic {
  /** Population mean. */
  mean: number;
  /** Population maximum. */
  max: number;
}
/** Named score and component fields retain stable meanings across policy choices. */
export type ExperimentScoreName = 'physicalWallet' | 'lifeNet' | 'generationGross' | 'generationNet' | 'cumulative' | 'bestLifePeak' | 'assessment';
/** Scaled positive component contributions; penalty is the positive cost subtracted. */
export type ExperimentFitnessComponent = 'survival' | 'food' | 'growth' | 'kills' | 'normalizedPoints' | 'topBonus' | 'penalty';
/** Explicit version-two historical winner assessment, separate from the old index. */
export interface HallOfFameAssessment {
  /** Fixed native assessment schema. */
  version: 2;
  /** Source population slot. */
  populationSlot: number;
  /** Full native physical ID as exact hexadecimal u64. */
  physicalId: string;
  /** Selected point assessment method. */
  pointMethod: PointAssessmentMethod;
  /** Actual nonpoint component source. */
  fitnessBundle: FitnessBundleMethod;
  /** Configured cumulative basis, even when another point method is active. */
  cumulativeGross: boolean;
  /** Exact selected life ordinal; zero denotes a cumulative bank. */
  selectedLifeOrdinal: string;
  /** Point value used by the fitness calculation. */
  assessment: number;
  /** Retained fitness input bundle. */
  bundle: { aliveSeconds: number; food: number; kills: string; retainedGrowth: number; peakLength: string };
  /** Economic inputs retain physical and individual meanings. */
  economics: { lifeNet: number; lifeGross: number; lifeBoostSpent: number; generationGross: number; generationNet: number; generationBoostSpent: number; physicalWallet: number };
  /** Exact counts; strings avoid silent browser counter narrowing. */
  lives: string;
  /** Finalized life count. */
  deaths: string;
  /** Successful replacement count. */
  respawns: string;
  /** Current/latest finalized life length. */
  latestLength: number;
  /** Latest life peak length. */
  latestLifePeakLength: number;
  /** Selected point life peak; zero for cumulative banks. */
  selectedLifePeakLength: number;
  /** Highest length reached by the individual in this generation. */
  generationPeakLength: number;
  /** Policy-selected length shown to users. */
  displayLength: number;
  /** Explicit label for the selected display length. */
  displayLengthKind: 'latest-life' | 'generation-peak' | 'selected-life-peak';
  /** Latest life ordinal, including unfinished lives. */
  currentLifeOrdinal: string;
}
/** Fixed-size completed-generation experiment projection for a bounded browser window. */
export interface GenerationExperimentSummary {
  /** Fixed native history schema. */
  version: 1;
  /** Exact completed step. */
  completedStep: string;
  /** Completion cause. */
  reason: 'manual' | 'duration' | 'early';
  /** Simulated elapsed seconds. */
  actualSeconds: number;
  /** Configured generation duration. */
  configuredSeconds: number;
  /** Population size used by reductions. */
  population: number;
  /** Exact totals. */
  lives: string;
  /** Finalized lives. */
  deaths: string;
  /** Successful replacement count. */
  respawns: string;
  /** Per-slot death distribution. */
  deathsPerSlot: { mean: number; median: number; max: string };
  /** Exclusive primary death counts. */
  primaryCauses: { wall: string; head: string; body: string; godMode: string };
  /** Total individual pending seconds. */
  pendingSeconds: number;
  /** Individuals with no remaining lives. */
  exhaustedSlots: number;
  /** All point projections, regardless of which assessment is active. */
  scores: Record<ExperimentScoreName, ExperimentStatistic>;
  /** Generation-wide economic totals. */
  economics: { gross: number; boostSpent: number; net: number };
  /** Actual corpse allocation outcomes. */
  corpse: { attemptedCount: string; emittedCount: string; discardedCount: string; attemptedValue: number; emittedValue: number; discardedValue: number };
  /** Native fitness component means/maxima, including penalty costs. */
  fitnessComponents: Record<ExperimentFitnessComponent, ExperimentStatistic>;
  /** Exact source configuration revision. */
  configRevision: string;
  /** Packed policy identity; no browser-owned calculation uses this value. */
  policyBits: string;
  /** Ordered economic operation count. */
  ledgerOperations: string;
  /** Actual evaluator used for this generation. */
  evaluator: 'life-assessments' | 'legacy-physical';
  /** The retained best-fitness winner, with explicit assessment/length semantics. */
  winner: HallOfFameAssessment;
}
