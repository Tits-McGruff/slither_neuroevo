// settings.ts
// UI slider specifications and helper functions for building and interacting
// with the settings panel.  This module does not render anything itself;
// rather, it populates a provided container element with controls defined
// by SETTING_SPECS and synchronises their values with the global CFG.

import { CFG } from './config.ts';
import { getByPath, setByPath, fmtNumber } from './utils.ts';

/** Supported input types for settings controls. */
type SettingControlType = 'range' | 'number' | 'checkbox' | 'action';

/** Settings specification describing a single CFG path control. */
interface SettingSpec {
  group: string;
  path?: string;
  label: string;
  min?: number;
  max?: number;
  step?: number;
  decimals?: number;
  requiresReset?: boolean;
  type?: SettingControlType;
  id?: string;
  actionLabel?: string;
  hint?: string;
  hintId?: string;
}

/** Input id for the baseline bot seed control. */
export const BASELINE_BOT_SEED_INPUT_ID = 'baselineBotSeed';
/** Hint id for invalid baseline bot seed values. */
export const BASELINE_BOT_SEED_HINT_ID = 'baselineBotSeedHint';
/** Button id for randomizing the baseline bot seed. */
export const BASELINE_BOT_SEED_RANDOMIZE_ID = 'baselineBotSeedRandomize';

/** Slider specifications used to build the settings UI. */
const SETTING_SPECS: SettingSpec[] = [
  { group: "World and food", path: "worldRadius", label: "World radius", min: 800, max: 10000, step: 50, decimals: 0, requiresReset: true },
  { group: "World and food", path: "pelletCountTarget", label: "Pellet target count", min: 100, max: 25000, step: 50, decimals: 0, requiresReset: true },
  { group: "World and food", path: "pelletSpawnPerSecond", label: "Pellet spawn per second", min: 5, max: 3500, step: 5, decimals: 0, requiresReset: true },
  { group: "World and food", path: "foodValue", label: "Food value per pellet", min: 0.1, max: 8.0, step: 0.1, decimals: 1, requiresReset: true },
  { group: "World and food", path: "growPerFood", label: "Growth per food", min: 0.1, max: 10.0, step: 0.1, decimals: 1, requiresReset: true },

  { group: "Baseline bots", path: "baselineBots.count", label: "Baseline bot count", min: 0, max: 120, step: 1, decimals: 0, requiresReset: true },
  { group: "Baseline bots", path: "baselineBots.respawnDelay", label: "Respawn delay (sec)", min: 0.5, max: 10.0, step: 0.5, decimals: 1, requiresReset: false },
  { group: "Baseline bots", path: "baselineBots.randomizeSeedPerGen", label: "Randomize base seed per generation", requiresReset: true, type: "checkbox" },
  { group: "Baseline bots", path: "baselineBots.seed", label: "Baseline bot base seed", min: 0, max: 4294967295, step: 1, decimals: 0, requiresReset: true, type: "number", id: BASELINE_BOT_SEED_INPUT_ID, hint: "Seed must be a non-negative integer.", hintId: BASELINE_BOT_SEED_HINT_ID },
  { group: "Baseline bots", label: "Randomize base seed", type: "action", actionLabel: "Randomize seed", id: BASELINE_BOT_SEED_RANDOMIZE_ID },

  { group: "Snake physics", path: "snakeBaseSpeed", label: "Base speed", min: 30, max: 650, step: 5, decimals: 0, requiresReset: true },
  { group: "Snake physics", path: "snakeBoostSpeed", label: "Boost speed (used as relative multiplier)", min: 40, max: 1200, step: 5, decimals: 0, requiresReset: true },
  { group: "Snake physics", path: "snakeTurnRate", label: "Turn rate", min: 0.4, max: 14.0, step: 0.1, decimals: 1, requiresReset: true },

  { group: "Snake physics", path: "snakeRadius", label: "Base radius", min: 3, max: 30, step: 1, decimals: 0, requiresReset: true },
  { group: "Snake physics", path: "snakeRadiusMax", label: "Max radius", min: 4, max: 50, step: 1, decimals: 0, requiresReset: true },
  { group: "Snake physics", path: "snakeThicknessScale", label: "Thickness scale", min: 0.0, max: 20.0, step: 0.1, decimals: 1, requiresReset: true },
  { group: "Snake physics", path: "snakeThicknessLogDiv", label: "Thickness log divisor", min: 1.0, max: 240.0, step: 1.0, decimals: 0, requiresReset: true },

  { group: "Snake physics", path: "snakeSpacing", label: "Segment spacing", min: 3.0, max: 20.0, step: 0.1, decimals: 1, requiresReset: true },
  { group: "Snake physics", path: "snakeStartLen", label: "Start length", min: 5, max: 140, step: 1, decimals: 0, requiresReset: true },
  { group: "Snake physics", path: "snakeMaxLen", label: "Max length", min: 60, max: 2800, step: 10, decimals: 0, requiresReset: true },
  { group: "Snake physics", path: "snakeMinLen", label: "Min length", min: 4, max: 80, step: 1, decimals: 0, requiresReset: true },

  { group: "Snake physics", path: "snakeSizeSpeedPenalty", label: "Size speed penalty", min: 0.0, max: 0.70, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Snake physics", path: "snakeBoostSizePenalty", label: "Size boost penalty", min: 0.0, max: 0.95, step: 0.01, decimals: 2, requiresReset: false },

  { group: "Boost and mass", path: "boost.minPointsToBoost", label: "Min points to boost", min: 0.0, max: 60.0, step: 0.1, decimals: 1, requiresReset: false },
  { group: "Boost and mass", path: "boost.pointsCostPerSecond", label: "Boost points cost per second", min: 0.0, max: 80.0, step: 0.5, decimals: 1, requiresReset: false },
  { group: "Boost and mass", path: "boost.pointsCostSizeFactor", label: "Boost cost size factor", min: 0.0, max: 4.0, step: 0.05, decimals: 2, requiresReset: false },
  { group: "Boost and mass", path: "boost.lenLossPerPoint", label: "Length loss per point", min: 0.0, max: 2.0, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Boost and mass", path: "boost.pelletValueFactor", label: "Boost drop pellet value factor", min: 0.0, max: 1.5, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Boost and mass", path: "boost.pelletJitter", label: "Boost drop jitter", min: 0.0, max: 80.0, step: 1.0, decimals: 0, requiresReset: false },

  { group: "Collision", path: "collision.substepMaxDt", label: "Substep max dt", min: 0.006, max: 0.05, step: 0.001, decimals: 3, requiresReset: false },
  { group: "Collision", path: "collision.skipSegments", label: "Skip segments near head", min: 0, max: 30, step: 1, decimals: 0, requiresReset: false },
  { group: "Collision", path: "collision.hitScale", label: "Hit scale", min: 0.45, max: 1.20, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Collision", path: "collision.cellSize", label: "Collision grid cell size", min: 20, max: 200, step: 1, decimals: 0, requiresReset: false },
  { group: "Collision", path: "collision.neighborRange", label: "Collision neighbor range", min: 1, max: 3, step: 1, decimals: 0, requiresReset: false },

  { group: "Evolution", path: "generationSeconds", label: "Generation duration seconds", min: 8, max: 240, step: 1, decimals: 0, requiresReset: true },
  { group: "Evolution", path: "eliteFrac", label: "Elite fraction", min: 0.01, max: 0.50, step: 0.01, decimals: 2, requiresReset: true },
  { group: "Evolution", path: "mutationRate", label: "Mutation rate", min: 0.0, max: 0.50, step: 0.005, decimals: 3, requiresReset: true },
  { group: "Evolution", path: "mutationStd", label: "Mutation std", min: 0.0, max: 2.50, step: 0.05, decimals: 2, requiresReset: true },
  { group: "Evolution", path: "crossoverRate", label: "Crossover rate", min: 0.0, max: 1.0, step: 0.02, decimals: 2, requiresReset: true },

  { group: "Observer and camera", path: "observer.focusRecheckSeconds", label: "Focus recheck seconds", min: 0.10, max: 6.0, step: 0.05, decimals: 2, requiresReset: false },
  { group: "Observer and camera", path: "observer.focusSwitchMargin", label: "Focus switch margin", min: 1.00, max: 1.60, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Observer and camera", path: "observer.earlyEndMinSeconds", label: "Early end min seconds", min: 0, max: 50, step: 1, decimals: 0, requiresReset: false },
  { group: "Observer and camera", path: "observer.earlyEndAliveThreshold", label: "Early end alive threshold", min: 1, max: 25, step: 1, decimals: 0, requiresReset: false },
  { group: "Observer and camera", path: "observer.overviewPadding", label: "Overview padding", min: 1.00, max: 1.80, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Observer and camera", path: "observer.zoomLerpFollow", label: "Follow zoom lerp", min: 0.0, max: 0.40, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Observer and camera", path: "observer.zoomLerpOverview", label: "Overview zoom lerp", min: 0.0, max: 0.40, step: 0.01, decimals: 2, requiresReset: false },
  { group: "Observer and camera", path: "observer.overviewExtraWorldMargin", label: "Overview extra margin", min: 0, max: 1200, step: 10, decimals: 0, requiresReset: false },

  { group: "Rewards", path: "reward.pointsPerFood", label: "Points per food", min: 0.0, max: 20.0, step: 0.1, decimals: 1, requiresReset: false },
  { group: "Rewards", path: "reward.pointsPerKill", label: "Points per kill", min: 0.0, max: 400.0, step: 1, decimals: 0, requiresReset: false },
  { group: "Rewards", path: "reward.pointsPerSecondAlive", label: "Points per second alive", min: 0.0, max: 10.0, step: 0.05, decimals: 2, requiresReset: false },

  { group: "Rewards", path: "reward.fitnessSurvivalPerSecond", label: "Fitness survival per second", min: 0.0, max: 10.0, step: 0.05, decimals: 2, requiresReset: false },
  { group: "Rewards", path: "reward.fitnessFood", label: "Fitness per food", min: 0.0, max: 80.0, step: 0.5, decimals: 1, requiresReset: false },
  { group: "Rewards", path: "reward.fitnessLengthPerSegment", label: "Fitness per grown segment", min: 0.0, max: 20.0, step: 0.05, decimals: 2, requiresReset: false },
  { group: "Rewards", path: "reward.fitnessKill", label: "Fitness per kill", min: 0.0, max: 400.0, step: 1, decimals: 0, requiresReset: false },
  { group: "Rewards", path: "reward.fitnessPointsNorm", label: "Fitness points normalization weight", min: 0.0, max: 300.0, step: 1, decimals: 0, requiresReset: false },
  { group: "Rewards", path: "reward.fitnessTopPointsBonus", label: "Fitness top points bonus", min: 0.0, max: 600.0, step: 1, decimals: 0, requiresReset: false },

  { group: "Brain and memory", path: "brain.gruHidden", label: "GRU hidden size", min: 4, max: 96, step: 1, decimals: 0, requiresReset: true },
  { group: "Brain and memory", path: "brain.lstmHidden", label: "LSTM hidden size", min: 4, max: 96, step: 1, decimals: 0, requiresReset: true },
  { group: "Brain and memory", path: "brain.rruHidden", label: "RRU hidden size", min: 4, max: 96, step: 1, decimals: 0, requiresReset: true },
  { group: "Brain and memory", path: "brain.controlDt", label: "Brain control dt", min: 0.008, max: 0.060, step: 0.001, decimals: 3, requiresReset: false },
  { group: "Brain and memory", path: "brain.gruMutationRate", label: "Recurrent mutation rate (GRU/LSTM/RRU)", min: 0.0, max: 0.35, step: 0.005, decimals: 3, requiresReset: true },
  { group: "Brain and memory", path: "brain.gruMutationStd", label: "Recurrent mutation std (GRU/LSTM/RRU)", min: 0.0, max: 1.60, step: 0.02, decimals: 2, requiresReset: true },
  { group: "Brain and memory", path: "brain.gruCrossoverMode", label: "Recurrent crossover mode (0 block, 1 unit)", min: 0, max: 1, step: 1, decimals: 0, requiresReset: true },
  { group: "Brain and memory", path: "brain.gruInitUpdateBias", label: "GRU init update gate bias (GRU only)", min: -2.5, max: 1.5, step: 0.05, decimals: 2, requiresReset: true },
  { group: "Brain and memory", path: "brain.lstmInitForgetBias", label: "LSTM init forget gate bias (LSTM only)", min: -1.5, max: 3.0, step: 0.05, decimals: 2, requiresReset: true },
  { group: "Brain and memory", path: "brain.rruInitGateBias", label: "RRU init gate bias (RRU only)", min: -1.5, max: 2.0, step: 0.05, decimals: 2, requiresReset: true },

  { group: "Misc", path: "dtClamp", label: "Frame dt clamp", min: 0.01, max: 0.12, step: 0.005, decimals: 3, requiresReset: false }
];

/**
 * Resolve the control type for a spec, defaulting to range sliders.
 * @param spec - Settings specification to inspect.
 * @returns Resolved control type.
 */
function resolveSpecType(spec: SettingSpec): SettingControlType {
  return spec.type ?? 'range';
}

/**
 * Read a settings input value as a number.
 * @param input - Input element to read.
 * @returns Numeric value, or null when invalid.
 */
function readInputValue(input: HTMLInputElement): number | null {
  if (input.type === 'checkbox') return input.checked ? 1 : 0;
  const path = input.dataset['path'];
  if (path === 'baselineBots.seed') {
    const parsed = Number(input.value);
    if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) return null;
    return Math.max(0, parsed);
  }
  const value = Number(input.value);
  if (!Number.isFinite(value)) return null;
  return value;
}

/**
 * Format a settings value for display.
 * @param input - Input element associated with the value.
 * @param value - Numeric value to format.
 * @returns Human-friendly formatted value string.
 */
function formatInputValue(input: HTMLInputElement, value: number): string {
  if (input.type === 'checkbox') return value ? 'On' : 'Off';
  const decimalsRaw = input.dataset['decimals'];
  const decimals = decimalsRaw ? Number(decimalsRaw) : 0;
  return fmtNumber(value, Number.isFinite(decimals) ? decimals : 0);
}

/**
 * Group the specifications by their group property into a map.
 * Used internally by buildSettingsUI to organize sliders into collapsible sections.
 * @returns Grouped setting specs keyed by group name.
 */
function groupSpecs(): Map<string, SettingSpec[]> {
  const m = new Map<string, SettingSpec[]>();
  for (const s of SETTING_SPECS) {
    if (!m.has(s.group)) m.set(s.group, []);
    m.get(s.group)!.push(s);
  }
  return m;
}

/**
 * Build the settings UI inside a given container element.
 * Each group becomes a details element containing slider controls for its
 * respective parameters. The caller is responsible for appending the container
 * to the DOM before invoking this function.
 * @param container - Container element to populate.
 */
export function buildSettingsUI(container: HTMLElement): void {
  container.innerHTML = "";
  const grouped = groupSpecs();
  for (const [groupName, specs] of grouped.entries()) {
    const det = document.createElement("details");
    det.open = false;
    const sum = document.createElement("summary");
    sum.textContent = groupName;
    det.appendChild(sum);
    const groupDiv = document.createElement("div");
    groupDiv.className = "group";
    for (const spec of specs) {
      const type = resolveSpecType(spec);
      const wrap = document.createElement("div");
      wrap.className = "setting";
      const topline = document.createElement("div");
      topline.className = "topline";
      const name = document.createElement("div");
      name.className = "name";
      name.textContent = spec.label;
      topline.appendChild(name);
      if (type !== 'action') {
        const value = document.createElement("div");
        value.className = "value";
        if (spec.path) {
          value.id = "val_" + spec.path.replaceAll(".", "_");
        }
        topline.appendChild(value);
      }
      wrap.appendChild(topline);

      if (type === 'action') {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "btn small";
        button.textContent = spec.actionLabel ?? spec.label;
        if (spec.id) button.id = spec.id;
        wrap.appendChild(button);
      } else {
        const input = document.createElement("input");
        if (type === 'checkbox') {
          input.type = "checkbox";
        } else if (type === 'number') {
          input.type = "number";
        } else {
          input.type = "range";
        }
        if (spec.min != null) input.min = String(spec.min);
        if (spec.max != null) input.max = String(spec.max);
        if (spec.step != null) input.step = String(spec.step);
        if (spec.path) input.dataset['path'] = spec.path;
        if (spec.decimals != null) input.dataset['decimals'] = String(spec.decimals);
        if (spec.requiresReset != null) {
          input.dataset['requiresReset'] = spec.requiresReset ? "1" : "0";
        }
        if (spec.id) input.id = spec.id;
        wrap.appendChild(input);
        if (spec.requiresReset != null) {
          const foot = document.createElement("div");
          foot.className = "foot";
          foot.innerHTML = spec.requiresReset
            ? `<span class="pill">reset</span> Applies on reset.`
            : `<span class="pill">live</span> Applies immediately and also on reset.`;
          wrap.appendChild(foot);
        }
      }

      if (spec.hint) {
        const hint = document.createElement("div");
        hint.className = "meta";
        hint.textContent = spec.hint;
        if (spec.hintId) hint.id = spec.hintId;
        wrap.appendChild(hint);
      }
      groupDiv.appendChild(wrap);
    }
    det.appendChild(groupDiv);
    container.appendChild(det);
  }
}

/**
 * Set all settings controls within the given root element to match CFG.
 * Also updates the displayed numeric values next to each control.
 * @param root - Root element containing the controls.
 */
export function applyValuesToSlidersFromCFG(root: HTMLElement): void {
  const inputs = root.querySelectorAll<HTMLInputElement>('input[data-path]');
  inputs.forEach(input => {
    const path = input.dataset['path']!;
    const rawValue = getByPath(CFG, path);
    const numericValue = typeof rawValue === 'number' ? rawValue : (rawValue ? 1 : 0);
    if (input.type === 'checkbox') {
      input.checked = Boolean(rawValue);
    } else {
      input.value = String(numericValue);
    }
    const out = document.getElementById("val_" + path.replaceAll(".", "_"));
    if (out) out.textContent = formatInputValue(input, numericValue);
  });
}

/**
 * Attach live update handlers to all sliders under the given root.
 * When the user drags a slider that does not require a reset, the global CFG
 * is updated immediately and the provided callback is invoked.
 * @param root - Root element containing the sliders.
 * @param onLiveUpdate - Callback invoked for live sliders.
 */
export function hookSliderEvents(
  root: HTMLElement,
  onLiveUpdate: (sliderEl: HTMLInputElement) => void
): void {
  const inputs = root.querySelectorAll<HTMLInputElement>('input[data-path]');
  inputs.forEach(input => {
    input.addEventListener("input", () => {
      const value = readInputValue(input);
      if (value != null) {
        const out = document.getElementById("val_" + input.dataset['path']!.replaceAll(".", "_"));
        if (out) out.textContent = formatInputValue(input, value);
      }
      if (input.dataset['requiresReset'] === "0" && value != null) {
        onLiveUpdate(input);
      }
    });
  });
}

/**
 * Persist the current slider values from the UI back into CFG.
 * This should be called whenever the user clicks "Apply" to commit changes.
 * Sliders that require a reset are not applied until a new world is constructed.
 * @param root - Root element containing the sliders.
 */
export function updateCFGFromUI(root: HTMLElement): void {
  const inputs = root.querySelectorAll<HTMLInputElement>('input[data-path]');
  inputs.forEach(input => {
    const path = input.dataset['path']!;
    const value = readInputValue(input);
    if (value == null) return;
    setByPath(CFG, path, value);
  });
}

/**
 * Orchestrate the full UI setup: build, apply values, and hook events.
 * Used by main.ts to initialize or reset the sidebar.
 * @param container - Container element to populate.
 * @param onLiveUpdate - Optional callback for live slider updates.
 */
export function setupSettingsUI(
  container: HTMLElement,
  onLiveUpdate?: (sliderEl: HTMLInputElement) => void
): void {
  buildSettingsUI(container);
  applyValuesToSlidersFromCFG(container);
  if (onLiveUpdate) hookSliderEvents(container, onLiveUpdate);
}
