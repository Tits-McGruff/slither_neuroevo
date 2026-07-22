// settings.ts
// UI slider specifications and helper functions for building and interacting
// with the settings panel.  This module does not render anything itself;
// rather, it populates a provided container element with controls defined
// by SETTING_SPECS and synchronises their values with the global CFG.

import { CFG, syncBrainInputSize } from './config.ts';
import { coerceSettingsUpdateValue, type SettingsPath } from './protocol/settings.ts';
import {
  BASELINE_BOT_SEED_HINT_ID,
  BASELINE_BOT_SEED_INPUT_ID,
  BASELINE_BOT_SEED_RANDOMIZE_ID,
  SETTING_DEFINITIONS,
  type SettingControlType,
  type SettingDefinition
} from './protocol/settingDefinitions.ts';
import { getByPath, setByPath, fmtNumber } from './utils.ts';

/** Preserve the public DOM ids historically exported by this module. */
export {
  BASELINE_BOT_SEED_HINT_ID,
  BASELINE_BOT_SEED_INPUT_ID,
  BASELINE_BOT_SEED_RANDOMIZE_ID
};

/** Settings specification alias used by the DOM builder. */
type SettingSpec = SettingDefinition;

/** Pure shared definitions used to build the settings UI. */
const SETTING_SPECS = SETTING_DEFINITIONS;

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
    const coerced = coerceSettingsUpdateValue(path as SettingsPath, value);
    setByPath(CFG, path, coerced);
  });
  syncBrainInputSize();
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
