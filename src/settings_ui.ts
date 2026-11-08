/** Core settings field projection and parsing, separate from session/reset ownership. */
import { parseDurationDraft } from './protocol/duration.ts';
import type { CoreSettings } from './protocol/settings.ts';
import { readLifeSettingInput,readSettingChoice,refreshSensorSettings,type SettingInputElement } from './settings.ts';

/** Existing core inputs supplied by the composition root. */
export interface CoreInputElements {
  readonly elSnakes: HTMLInputElement;
  readonly elSimSpeed: HTMLInputElement;
  readonly elLayers: HTMLInputElement;
  readonly elN1: HTMLInputElement;
  readonly elN2: HTMLInputElement;
  readonly elN3: HTMLInputElement;
  readonly elN4: HTMLInputElement;
  readonly elN5: HTMLInputElement;
}
/** Existing value labels supplied without copying mutable settings. */
export interface CoreValueLabels {
  readonly snakesVal: HTMLElement;
  readonly simSpeedVal: HTMLElement;
  readonly layersVal: HTMLElement;
  readonly n1Val: HTMLElement;
  readonly n2Val: HTMLElement;
  readonly n3Val: HTMLElement;
  readonly n4Val: HTMLElement;
  readonly n5Val: HTMLElement;
}
/** The historical core slider request shape. */
export type CoreUiDraft = Pick<CoreSettings, 'snakeCount' | 'simSpeed' | 'hiddenLayers' | 'neurons1' | 'neurons2' | 'neurons3' | 'neurons4' | 'neurons5'>;
/** UI-only lock projection; the root still owns the flag and reset callbacks. */
export interface SettingsLockPort {
  readonly settingsTab: HTMLElement | null;
  readonly settingsControls: HTMLElement | null;
  readonly toggleSettingsLock: HTMLElement | null;
  readonly settingsLockHint: HTMLElement | null;
  readonly settingsLocked: boolean;
  readonly resolveSettingsRoot: () => HTMLElement;
  readonly refreshCoreUIState: () => void;
}


/**
 * Reads the core UI slider values into a settings object for the world.
 */
export function readSettingsFromCoreUI(inputs: CoreInputElements): CoreUiDraft {
  const { elSnakes, elSimSpeed, elLayers, elN1, elN2, elN3, elN4, elN5 } = inputs;
  const hiddenLayers = parseInt(elLayers.value, 10);
  return {
    snakeCount: parseInt(elSnakes.value, 10),
    simSpeed: parseFloat(elSimSpeed.value),
    hiddenLayers,
    neurons1: parseInt(elN1.value, 10),
    neurons2: parseInt(elN2.value, 10),
    neurons3: parseInt(elN3.value, 10),
    neurons4: parseInt(elN4.value, 10),
    neurons5: parseInt(elN5.value, 10)
  };
}


/**
 * Apply core settings values to the UI sliders.
 * @param settings - Partial core settings to apply to the UI.
 */
export function applyCoreSettingsToUi(settings: Partial<CoreSettings>, inputs: CoreInputElements, refreshCoreUIState: () => void): void {
  const { elSnakes, elSimSpeed, elLayers, elN1, elN2, elN3, elN4, elN5 } = inputs;
  if (Number.isFinite(settings.snakeCount)) {
    elSnakes.value = String(settings.snakeCount);
  }
  if (Number.isFinite(settings.simSpeed)) {
    elSimSpeed.value = String(settings.simSpeed);
  }
  if (Number.isFinite(settings.hiddenLayers)) {
    elLayers.value = String(settings.hiddenLayers);
  }
  if (Number.isFinite(settings.neurons1)) {
    elN1.value = String(settings.neurons1);
  }
  if (Number.isFinite(settings.neurons2)) {
    elN2.value = String(settings.neurons2);
  }
  if (Number.isFinite(settings.neurons3)) {
    elN3.value = String(settings.neurons3);
  }
  if (Number.isFinite(settings.neurons4)) {
    elN4.value = String(settings.neurons4);
  }
  if (Number.isFinite(settings.neurons5)) {
    elN5.value = String(settings.neurons5);
  }
  refreshCoreUIState();
}


/**
 * Apply graph mode styling to stack slider rows and hints.
 * @param graphActive - Whether a custom graph spec is active.
 */
function applyGraphModeUiState(graphActive: boolean, inputs: CoreInputElements, graphModeHint: HTMLElement | null): void {
  const { elLayers, elN1, elN2, elN3, elN4, elN5 } = inputs;
  const sliders = [elLayers, elN1, elN2, elN3, elN4, elN5];
  sliders.forEach(slider => {
    const row = slider.closest('.row');
    if (!row) return;
    row.classList.toggle('graph-stack-disabled', graphActive);
  });
  if (graphModeHint) {
    graphModeHint.textContent = graphActive
      ? 'Custom graph active; stack sliders are ignored.'
      : '';
  }
}


/**
 * Synchronises the displayed numbers next to the core UI sliders and
 * disables or enables the neuron sliders based on the number of hidden
 * layers.
 */
export function refreshCoreUIState(inputs: CoreInputElements, labels: CoreValueLabels, isLocked: () => boolean, isGraphActive: () => boolean, graphModeHint: HTMLElement | null): void {
  const { elSnakes, elSimSpeed, elLayers, elN1, elN2, elN3, elN4, elN5 } = inputs;
  const { snakesVal, simSpeedVal, layersVal, n1Val, n2Val, n3Val, n4Val, n5Val } = labels;
  snakesVal.textContent = elSnakes.value;
  simSpeedVal.textContent = Number(elSimSpeed.value).toFixed(2);
  layersVal.textContent = elLayers.value;
  n1Val.textContent = elN1.value;
  n2Val.textContent = elN2.value;
  n3Val.textContent = elN3.value;
  n4Val.textContent = elN4.value;
  n5Val.textContent = elN5.value;
  const L = parseInt(elLayers.value, 10);
  const graphActive = isGraphActive();
  const disableAll = isLocked() || graphActive;
  elLayers.disabled = disableAll;
  elN1.disabled = disableAll;
  elN2.disabled = disableAll || L < 2;
  elN3.disabled = disableAll || L < 3;
  elN4.disabled = disableAll || L < 4;
  elN5.disabled = disableAll || L < 5;
  applyGraphModeUiState(graphActive, inputs, graphModeHint);
  const applyOpacity = (el: HTMLInputElement) => {
    el.style.opacity = el.disabled ? '0.45' : '1';
  };
  applyOpacity(elN1);
  applyOpacity(elN2);
  applyOpacity(elN3);
  applyOpacity(elN4);
  applyOpacity(elN5);
  applyOpacity(elLayers);
}


/**
 * Parse a baseline bot seed from a raw input string.
 * @param value - Raw input value.
 * @returns Normalized seed value or null when invalid.
 */
export function parseBaselineSeedValue(value: string): number | null {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || !Number.isInteger(parsed)) return null;
  return Math.max(0, parsed);
}


/**
 * Read a numeric value from a CFG-backed settings input.
 * @param input - Input element to read.
 * @returns Parsed numeric value or null when invalid.
 */
export function readSettingsInputValue(input: SettingInputElement): number | null {
  if (input.dataset['lifeContract'] === '1') return readLifeSettingInput(input);
  if (input.dataset['controlType'] === 'select') return readSettingChoice(input as HTMLSelectElement);
  if (input.dataset['controlType'] === 'duration') return parseDurationDraft(input.value, input.dataset['recordedDuration']);
  if (input.type === "checkbox" && 'checked' in input) return input.checked ? 1 : 0;
  if (input.dataset['path'] === 'baselineBots.seed') {
    return parseBaselineSeedValue(input.value);
  }
  const value = Number(input.value);
  if (!Number.isFinite(value)) return null;
  return value;
}


/**
 * Apply the settings lock state to UI controls.
 */
export function applySettingsLock(ports: SettingsLockPort): void {
  const { settingsTab, settingsControls, toggleSettingsLock, settingsLockHint } = ports;
  if (!settingsTab || !settingsControls || !toggleSettingsLock) return;
  settingsTab.classList.toggle('settings-locked', ports.settingsLocked);
  toggleSettingsLock.textContent = ports.settingsLocked ? 'Unlock' : 'Lock';
  if (settingsLockHint) {
    settingsLockHint.textContent = ports.settingsLocked
      ? 'Configuration protected. Viewing, camera controls and export remain available.'
      : 'Live controls apply immediately. Reset controls are drafts until Apply and reset.';
  }
  const controls = settingsControls.querySelectorAll('input, button, select, textarea');
  controls.forEach(control => {
    (control as HTMLInputElement).disabled = ports.settingsLocked;
  });
  refreshSensorSettings(ports.resolveSettingsRoot(), ports.settingsLocked);
  if (!ports.settingsLocked) ports.refreshCoreUIState();
}

/** Surface pending reset drafts even while their configuration groups are collapsed. */
export function showResetDraft(pending: boolean): void {
  const notice = document.getElementById('pendingConfiguration');
  if (notice) {
    notice.hidden = !pending;
    notice.textContent = pending ? 'Pending configuration changes. Stage graph edits, then Apply and reset to replace the population using the same seed.' : '';
  }
  if (!pending) for (const group of document.querySelectorAll('details.has-draft')) group.classList.remove('has-draft');
}
