// settings.ts
// UI slider specifications and helper functions for building and interacting
// with the settings panel.  This module does not render anything itself;
// rather, it populates a provided container element with controls defined
// by SETTING_SPECS and synchronises their values with the global CFG.

import { CFG } from './config.js';
import { getByPath, setByPath, fmtNumber } from './utils.js';

interface SettingSpec {
  group: string;
  path: string;
  label: string;
  min: number;
  max: number;
  step: number;
  decimals: number;
  requiresReset: boolean;
}

// Each entry in SETTING_SPECS describes a slider.  See the original
// slither_neuroevo.html for more details on the range and meaning of
// individual parameters.
const SETTING_SPECS: SettingSpec[] = [
  { group: "World and food", path: "worldRadius", label: "World radius", min: 800, max: 10000, step: 50, decimals: 0, requiresReset: true },
  { group: "World and food", path: "pelletCountTarget", label: "Pellet target count", min: 100, max: 25000, step: 50, decimals: 0, requiresReset: true },
  { group: "World and food", path: "pelletSpawnPerSecond", label: "Pellet spawn per second", min: 5, max: 3500, step: 5, decimals: 0, requiresReset: true },
  { group: "World and food", path: "foodValue", label: "Food value per pellet", min: 0.1, max: 8.0, step: 0.1, decimals: 1, requiresReset: true },
  { group: "World and food", path: "growPerFood", label: "Growth per food", min: 0.1, max: 10.0, step: 0.1, decimals: 1, requiresReset: true },

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

  { group: "Brain and memory", path: "brain.useGRU", label: "Use GRU memory (0/1)", min: 0, max: 1, step: 1, decimals: 0, requiresReset: true },
  { group: "Brain and memory", path: "brain.gruHidden", label: "GRU hidden size", min: 4, max: 96, step: 1, decimals: 0, requiresReset: true },
  { group: "Brain and memory", path: "brain.controlDt", label: "Brain control dt", min: 0.008, max: 0.060, step: 0.001, decimals: 3, requiresReset: false },
  { group: "Brain and memory", path: "brain.gruMutationRate", label: "GRU mutation rate", min: 0.0, max: 0.35, step: 0.005, decimals: 3, requiresReset: true },
  { group: "Brain and memory", path: "brain.gruMutationStd", label: "GRU mutation std", min: 0.0, max: 1.60, step: 0.02, decimals: 2, requiresReset: true },
  { group: "Brain and memory", path: "brain.gruCrossoverMode", label: "GRU crossover mode (0 block, 1 unit)", min: 0, max: 1, step: 1, decimals: 0, requiresReset: true },
  { group: "Brain and memory", path: "brain.gruInitUpdateBias", label: "GRU init update gate bias", min: -2.5, max: 1.5, step: 0.05, decimals: 2, requiresReset: true },

  { group: "Misc", path: "dtClamp", label: "Frame dt clamp", min: 0.01, max: 0.12, step: 0.005, decimals: 3, requiresReset: false }
];

/**
 * Groups the specifications by their "group" property into a map.  Used
 * internally by buildSettingsUI to organise sliders into collapsible
 * sections.
 * @returns {Map<string, Array<Object>>}
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
 * Builds the settings UI inside a given container element.  Each group
 * becomes a <details> element containing slider controls for its
 * respective parameters.  The caller is responsible for appending the
 * container to the DOM before invoking this function.
 * @param {HTMLElement} container
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
      const wrap = document.createElement("div");
      wrap.className = "setting";
      const topline = document.createElement("div");
      topline.className = "topline";
      const name = document.createElement("div");
      name.className = "name";
      name.textContent = spec.label;
      const value = document.createElement("div");
      value.className = "value";
      value.id = "val_" + spec.path.replaceAll(".", "_");
      topline.appendChild(name);
      topline.appendChild(value);
      const slider = document.createElement("input");
      slider.type = "range";
      slider.min = String(spec.min);
      slider.max = String(spec.max);
      slider.step = String(spec.step);
      slider.dataset.path = spec.path;
      slider.dataset.decimals = String(spec.decimals ?? 2);
      slider.dataset.requiresReset = spec.requiresReset ? "1" : "0";
      const foot = document.createElement("div");
      foot.className = "foot";
      foot.innerHTML = spec.requiresReset
        ? `<span class="pill">reset</span> Applies on reset.`
        : `<span class="pill">live</span> Applies immediately and also on reset.`;
      wrap.appendChild(topline);
      wrap.appendChild(slider);
      wrap.appendChild(foot);
      groupDiv.appendChild(wrap);
    }
    det.appendChild(groupDiv);
    container.appendChild(det);
  }
}

/**
 * Sets all slider controls within the given root element to match the
 * current values in CFG.  Also updates the displayed numeric values next
 * to each slider.
 * @param {HTMLElement} root
 */
export function applyValuesToSlidersFromCFG(root: HTMLElement): void {
  const sliders = root.querySelectorAll<HTMLInputElement>('input[type="range"][data-path]');
  sliders.forEach(sl => {
    const path = sl.dataset.path!;
    const v = getByPath(CFG, path);
    sl.value = String(v);
    const decimals = parseInt(sl.dataset.decimals!, 10);
    const out = document.getElementById("val_" + path.replaceAll(".", "_"));
    if (out) out.textContent = fmtNumber(Number(sl.value), decimals);
  });
}

/**
 * Attaches live update handlers to all sliders under the given root.
 * When the user drags a slider that does not require a reset, the global
 * CFG is updated immediately and the provided callback is invoked.
 * @param {HTMLElement} root
 * @param {Function} onLiveUpdate
 */
export function hookSliderEvents(
  root: HTMLElement,
  onLiveUpdate: (sliderEl: HTMLInputElement) => void
): void {
  const sliders = root.querySelectorAll<HTMLInputElement>('input[type="range"][data-path]');
  sliders.forEach(sl => {
    sl.addEventListener("input", () => {
      const decimals = parseInt(sl.dataset.decimals!, 10);
      const out = document.getElementById("val_" + sl.dataset.path!.replaceAll(".", "_"));
      if (out) out.textContent = fmtNumber(Number(sl.value), decimals);
      if (sl.dataset.requiresReset === "0") onLiveUpdate(sl);
    });
  });
}

/**
 * Persists the current slider values from the UI back into CFG.  This
 * should be called whenever the user clicks "Apply" to commit their
 * changes.  Sliders that require a reset are not applied until a new
 * world is constructed.
 * @param {HTMLElement} root
 */
export function updateCFGFromUI(root: HTMLElement): void {
  const sliders = root.querySelectorAll<HTMLInputElement>('input[type="range"][data-path]');
  sliders.forEach(sl => setByPath(CFG, sl.dataset.path!, Number(sl.value)));
}

/**
 * Orchestrates the full UI setup: build, apply values, and hook events.
 * Used by main.ts to init or reset the sidebar.
 * @param {HTMLElement} container 
 * @param {Function} onLiveUpdate 
 */
export function setupSettingsUI(
  container: HTMLElement,
  onLiveUpdate?: (sliderEl: HTMLInputElement) => void
): void {
  buildSettingsUI(container);
  applyValuesToSlidersFromCFG(container);
  if (onLiveUpdate) hookSliderEvents(container, onLiveUpdate);
}
