// settings.ts
// UI slider specifications and helper functions for building and interacting
// with the settings panel.  This module does not render anything itself;
// rather, it populates a provided container element with controls defined
// by SETTING_SPECS and synchronises their values with the global CFG.

import { CFG, CFG_DEFAULT, syncBrainInputSize } from './config.ts';
import { formatDuration, parseDurationDraft } from './protocol/duration.ts';
import { coerceSettingsUpdateValue, encodeSettingsUpdateValue, type SettingsPath, type SettingsUpdate } from './protocol/settings.ts';
import {
  BASELINE_BOT_SEED_HINT_ID,
  BASELINE_BOT_SEED_INPUT_ID,
  BASELINE_BOT_SEED_RANDOMIZE_ID,
  SETTING_DEFINITIONS, SETTING_DEFINITION_BY_PATH, normalizeSettingValue,
  EVOLVED_SETTING_PATHS, EVOLVED_RESPAWN_PRESET_ID,
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

/** Numeric inputs and closed-choice controls share the same atomic draft flow. */
export type SettingInputElement = HTMLInputElement | HTMLSelectElement;

/**
 * Pure shared definitions used to build the settings UI.
 * The sensor logger and automatic focus controls belong to the retained
 * retired authoritative camera and now remain browser presentation controls.
 */
const SETTING_SPECS = SETTING_DEFINITIONS.filter(spec =>
  !spec.hidden && spec.path !== 'sense.debug' && spec.path !== 'observer.focusRecheckSeconds' &&
  spec.path !== 'observer.focusSwitchMargin');

/** Camera presentation values consumed by the browser, excluded from server commands. */
export const BROWSER_CAMERA_SETTING_PATHS: ReadonlySet<string> = new Set([
  'observer.overviewPadding',
  'observer.zoomLerpFollow',
  'observer.zoomLerpOverview',
  'observer.overviewExtraWorldMargin'
]);

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
function readInputValue(input: SettingInputElement): number | null {
  if (input.dataset['lifeContract'] === '1') return readLifeSettingInput(input);
  if (input.dataset['controlType'] === 'select') return readSettingChoice(input as HTMLSelectElement);
  if (input.type === 'checkbox' && 'checked' in input) return input.checked ? 1 : 0;
  const path = input.dataset['path'];
  if (input.dataset['controlType'] === 'duration') return parseDurationDraft(input.value, input.dataset['recordedDuration']);
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
function formatInputValue(input: SettingInputElement, value: number): string {
  if (input.dataset['controlType'] === 'select') {
    const option = Array.from((input as HTMLSelectElement).options).find(option => option.value === String(value));
    return option?.label || option?.textContent || String(value);
  }
  if (input.type === 'checkbox') return value ? 'On' : 'Off';
  if (input.dataset['controlType'] === 'duration') return formatDuration(value);
  const decimalsRaw = input.dataset['decimals'];
  const decimals = decimalsRaw ? Number(decimalsRaw) : 0;
  return fmtNumber(value, Number.isFinite(decimals) ? decimals : 0);
}

/** Validate life choices and numeric bounds without rounding or clamping a draft. */
export function readLifeSettingInput(input: SettingInputElement): number | null {
  const definition = SETTING_DEFINITION_BY_PATH.get(input.dataset['path'] ?? '');
  if (!definition?.lifeContract) return null;
  const value = input.type === 'checkbox' && 'checked' in input ? Number(input.checked)
    : input.value.trim() === '' ? NaN : Number(input.value);
  try { return normalizeSettingValue(definition, value); } catch { return null; }
}

/** Read only a declared integer option; empty/unknown choices never become zero. */
export function readSettingChoice(input: HTMLSelectElement): number | null {
  if (!Array.from(input.options).some(option => option.value === input.value)) return null;
  const value = Number(input.value);
  return input.value !== '' && Number.isSafeInteger(value) ? value : null;
}

/** Create one numeric/choice control with the shared path and reset metadata. */
export function createSettingControl(spec: SettingDefinition): SettingInputElement {
  const type = resolveSpecType(spec);
  let input: SettingInputElement;
  if (type === 'select') {
    const choices = spec.options;
    if (!choices?.length || choices.some(choice => !Number.isSafeInteger(choice.value)) ||
        new Set(choices.map(choice => choice.value)).size !== choices.length) {
      throw new Error('Select settings require distinct integer options.');
    }
    const select = document.createElement('select');
    for (const choice of choices) {
      const option = document.createElement('option');
      option.value = String(choice.value); option.textContent = choice.label;
      select.appendChild(option);
    }
    select.dataset['controlType'] = 'select'; input = select;
  } else {
    if (type === 'action') throw new Error('Action settings use a button.');
    const numeric = document.createElement('input');
    numeric.type = type === 'duration' ? 'text' : type;
    if (type === 'duration') { numeric.maxLength = 128; numeric.dataset['controlType'] = 'duration'; }
    if (spec.min != null) numeric.min = String(spec.min);
    if (spec.max != null) numeric.max = String(spec.max);
    if (spec.step != null) numeric.step = String(spec.step);
    input = numeric;
  }
  if (spec.path) input.dataset['path'] = spec.path;
  if (spec.lifeContract) input.dataset['lifeContract'] = '1';
  if (spec.decimals != null) input.dataset['decimals'] = String(spec.decimals);
  if (spec.requiresReset != null) input.dataset['requiresReset'] = spec.requiresReset ? '1' : '0';
  if (spec.id) input.id = spec.id;
  return input;
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
        const input = createSettingControl(spec);
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
 * @param paths - Optional subset, preserving unrelated pending drafts.
 * @param locked - Preserve the panel lock for conditional controls.
 */
export function applyValuesToSlidersFromCFG(root: HTMLElement, paths?: ReadonlySet<string>, locked = false): void {
  const inputs = root.querySelectorAll<SettingInputElement>('input[data-path], select[data-path]');
  inputs.forEach(input => {
    const path = input.dataset['path']!;
    if (paths && !paths.has(path)) return;
    const rawValue = getByPath(CFG, path);
    const numericValue = encodeSettingsUpdateValue(path, rawValue);
    if (input.dataset['controlType'] === 'duration') input.dataset['recordedDuration'] = String(numericValue);
    if (input.type === 'checkbox' && 'checked' in input) {
      input.checked = Boolean(rawValue);
    } else {
      input.value = input.dataset['controlType'] === 'duration'
        ? formatDuration(numericValue) : String(numericValue);
    }
    const out = document.getElementById("val_" + path.replaceAll(".", "_"));
    if (out) out.textContent = formatInputValue(input, numericValue);
  });
  refreshSensorSettings(root, locked);
}

/** Read pending selectors without changing the active browser configuration. */
function sensorSelections(root: HTMLElement): {version: number; algorithm: number} {
  const inputs = Array.from(root.querySelectorAll<SettingInputElement>('input[data-path], select[data-path]'));
  return {version: Number(inputs.find(input => input.dataset['path'] === 'sense.layoutVersion')?.value ??
    encodeSettingsUpdateValue('sense.layoutVersion', CFG.sense.layoutVersion)),
  algorithm: Number(inputs.find(input => input.dataset['path'] === 'baselineBots.algorithm')?.value ?? CFG.baselineBots.algorithm)};
}

/** Inactive choices remain drafts and are never submitted as runtime settings. */
function inactiveSensorSetting(path: string, version: number, algorithm: number): boolean {
  return ((path === 'sense.profile' || path.startsWith('sense.ray')) && version !== 4) ||
    ((path === 'baselineBots.behaviorMode' || path === 'baselineBots.controlDt') && (algorithm !== 2 || version !== 4));
}

/** Keep v4/v2-only controls disabled with a visible explanation, including after unlocking. */
export function refreshSensorSettings(root: HTMLElement, locked = false): void {
  const {version, algorithm} = sensorSelections(root);
  for (const input of root.querySelectorAll<SettingInputElement>('input[data-path], select[data-path]')) {
    const path = input.dataset['path']!;
    if (path !== 'sense.profile' && !path.startsWith('sense.ray') &&
        path !== 'baselineBots.behaviorMode' && path !== 'baselineBots.controlDt') continue;
    const inactive = inactiveSensorSetting(path, version, algorithm);
    input.disabled = locked || inactive;
    input.title = inactive ? (path.startsWith('sense.') ? 'Select v4 sensors to use this setting.' : 'Select v4 sensors and baseline v2 to use this setting.') : '';
  }
}

/** Collect one reset draft, excluding camera presentation and inactive contract fields. */
export function collectSettingsUpdates(root: HTMLElement): SettingsUpdate[] {
  const {version, algorithm} = sensorSelections(root);
  const updates: SettingsUpdate[] = [];
  for (const input of root.querySelectorAll<SettingInputElement>('input[data-path], select[data-path]')) {
    const path = input.dataset['path']!;
    if (BROWSER_CAMERA_SETTING_PATHS.has(path) || inactiveSensorSetting(path, version, algorithm) ||
        (EVOLVED_SETTING_PATHS.has(path) && CFG.evolvedRespawn.contractVersion === 0)) continue;
    const value = readInputValue(input);
    if (value !== null) updates.push({path: path as SettingsPath, value});
  }
  if (version === 4) updates.push({path: 'sense.rayMaximumDue', value: CFG.sense.rayMaximumDue});
  if (CFG.evolvedRespawn.contractVersion === 1) updates.push({path: 'evolvedRespawn.contractVersion', value: 1});
  return updates;
}

/** Select the explicit life preset without overwriting other unsubmitted drafts. */
export function stageEvolvedRespawnPreset(root: HTMLElement): void {
  for (const path of EVOLVED_SETTING_PATHS) setByPath(CFG, path, getByPath(CFG_DEFAULT, path));
  CFG.evolvedRespawn.enabled = true;
  CFG.world.pelletOverflow = 1;
  applyValuesToSlidersFromCFG(root, EVOLVED_SETTING_PATHS);
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
  onLiveUpdate: (sliderEl: SettingInputElement) => void
): void {
  const inputs = root.querySelectorAll<SettingInputElement>('input[data-path], select[data-path]');
  inputs.forEach(input => {
    input.addEventListener("input", () => {
      if (input.dataset['path'] === 'sense.layoutVersion' || input.dataset['path'] === 'baselineBots.algorithm') refreshSensorSettings(root);
      const value = readInputValue(input);
      if (value != null && input.dataset['lifeContract'] === '1') CFG.evolvedRespawn.contractVersion = 1;
      if (input.dataset['lifeContract'] === '1') {
        input.setCustomValidity(value == null ? 'Enter a valid value or choose a listed option.' : '');
        input.setAttribute('aria-invalid', String(value == null));
      }
      if (input.dataset['controlType'] === 'select') {
        input.setCustomValidity(value == null ? 'Choose one of the listed options.' : '');
        input.setAttribute('aria-invalid', String(value == null));
      }
      if (input.dataset['controlType'] === 'duration') {
        input.setCustomValidity(value == null ? 'Enter a duration from 8 seconds to 7 days.' : '');
        input.setAttribute('aria-invalid', String(value == null));
      }
      if (value != null) {
        const out = document.getElementById("val_" + input.dataset['path']!.replaceAll(".", "_"));
        if (out) out.textContent = formatInputValue(input, value);
      }
      if (input.dataset['requiresReset'] === "0" && value != null) {
        onLiveUpdate(input);
      }
    });
  });
  document.getElementById(EVOLVED_RESPAWN_PRESET_ID)?.addEventListener?.('click', () => stageEvolvedRespawnPreset(root));
}

/**
 * Persist the current slider values from the UI back into CFG.
 * This should be called whenever the user clicks "Apply" to commit changes.
 * Sliders that require a reset are not applied until a new world is constructed.
 * @param root - Root element containing the sliders.
 */
export function updateCFGFromUI(root: HTMLElement): void {
  if (!validateSettingDrafts(root)) return;
  const {version, algorithm} = sensorSelections(root);
  const inputs = root.querySelectorAll<SettingInputElement>('input[data-path], select[data-path]');
  inputs.forEach(input => {
    const path = input.dataset['path']!;
    if (inactiveSensorSetting(path, version, algorithm)) return;
    if (EVOLVED_SETTING_PATHS.has(path) && CFG.evolvedRespawn.contractVersion === 0) return;
    const value = readInputValue(input);
    if (value == null) return;
    const coerced = coerceSettingsUpdateValue(path as SettingsPath, value);
    setByPath(CFG, path, coerced);
  });
  syncBrainInputSize();
}

/**
 * Validate duration and closed-choice drafts before applying any reset settings.
 * @param root - Settings control container.
 * @returns Whether all supported drafts are valid; invalid controls show feedback.
 */
export function validateSettingDrafts(root: HTMLElement): boolean {
  const inputs = root.querySelectorAll<SettingInputElement>('input[data-path], select[data-path]');
  const {version, algorithm} = sensorSelections(root);
  let valid = true;
  inputs.forEach(input => {
    const path = input.dataset['path']!;
    if (inactiveSensorSetting(path, version, algorithm)) return;
    let contractError = '';
    if (path === 'baselineBots.algorithm' && algorithm === 2 && version !== 4) contractError = 'Baseline v2 requires v4 sensors.';
    if (path.startsWith('sense.ray') || path === 'baselineBots.controlDt') {
      const value = input.value.trim() === '' ? NaN : Number(input.value);
      const definition = SETTING_DEFINITION_BY_PATH.get(path)!;
      if (!Number.isFinite(value) || (definition.valueType === 'integer' && !Number.isSafeInteger(value)) ||
          value < definition.min! || value > definition.max!) contractError = 'Enter a value within the displayed bounds.';
      if (path === 'sense.rayMinimum' && !contractError) {
        const cap = Number(Array.from(inputs).find(candidate => candidate.dataset['path'] === 'sense.rayCap')?.value ?? CFG.sense.rayCap);
        const batch = Number(Array.from(inputs).find(candidate => candidate.dataset['path'] === 'sense.rayBatch')?.value ?? CFG.sense.rayBatch);
        if (value > cap || value * CFG.sense.rayMaximumDue > batch) contractError = 'Ray minimum must fit the cap and the batch budget for all admitted owners.';
      }
    }
    if (contractError) {
      input.setCustomValidity(contractError); input.setAttribute('aria-invalid', 'true');
      if (valid) input.reportValidity(); valid = false; return;
    }
    if (path.startsWith('sense.ray') || path === 'baselineBots.controlDt' || path === 'baselineBots.algorithm') {
      input.setCustomValidity(''); input.setAttribute('aria-invalid', 'false');
    }
    if (input.dataset['lifeContract'] === '1') {
      const accepted = readLifeSettingInput(input) != null;
      input.setCustomValidity(accepted ? '' : 'Enter a valid value or choose a listed option.');
      input.setAttribute('aria-invalid', String(!accepted));
      if (!accepted && valid) input.reportValidity();
      valid = valid && accepted; return;
    }
    if (input.dataset['controlType'] === 'select') {
      const accepted = readSettingChoice(input as HTMLSelectElement) != null;
      input.setCustomValidity(accepted ? '' : 'Choose one of the listed options.');
      input.setAttribute('aria-invalid', String(!accepted));
      if (!accepted && valid) input.reportValidity();
      valid = valid && accepted; return;
    }
    if (input.dataset['controlType'] !== 'duration') return;
    const accepted = parseDurationDraft(input.value, input.dataset['recordedDuration']) != null;
    input.setCustomValidity(accepted ? '' : 'Enter a duration from 8 seconds to 7 days.');
    input.setAttribute('aria-invalid', String(!accepted));
    if (!accepted && valid) input.reportValidity();
    valid = valid && accepted;
  });
  return valid;
}

/**
 * Orchestrate the full UI setup: build, apply values, and hook events.
 * Used by main.ts to initialize or reset the sidebar.
 * @param container - Container element to populate.
 * @param onLiveUpdate - Optional callback for live slider updates.
 */
export function setupSettingsUI(
  container: HTMLElement,
  onLiveUpdate?: (sliderEl: SettingInputElement) => void
): void {
  buildSettingsUI(container);
  applyValuesToSlidersFromCFG(container);
  if (onLiveUpdate) hookSliderEvents(container, onLiveUpdate);
}
