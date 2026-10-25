import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  buildSettingsUI,
  applyValuesToSlidersFromCFG,
  hookSliderEvents,
  updateCFGFromUI,
  createSettingControl,
  readSettingChoice, stageEvolvedRespawnPreset, collectSettingsUpdates, refreshSensorSettings, validateSettingDrafts
} from './settings.ts';
import { CFG } from './config.ts';
import { fmtNumber } from './utils.ts';
import { EVOLVED_SETTING_PATHS } from './protocol/settingDefinitions.ts';

/** Minimal DOM element stub for settings UI tests. */
class FakeElement {
  /** Browser form state and validation text used by conditional sensor controls. */
  disabled = false;
  /** Checked state for life and baseline flags. */
  checked = false;
  /** Most recent browser validation message. */
  validationMessage = '';
  /** Select options retain their declared integer wire values. */
  get options(): FakeElement[] { return this.children.filter(child => child.tagName === 'option'); }
  /** Keep validation observable without a real DOM. */
  setCustomValidity(message: string): void { this.validationMessage = message; }
  /** Attribute styling is outside this bounded form stub. */
  setAttribute(): void {}
  /** Validation reporting needs no interactive browser in this test. */
  reportValidity(): void {}
  /** Element tag name. */
  tagName: string;
  /** Child element list. */
  children: FakeElement[];
  /** Data attributes map. */
  dataset: Record<string, string>;
  /** CSS class string. */
  className: string;
  /** Element id. */
  id: string;
  /** Text content value. */
  textContent: string;
  /** Inner HTML value. */
  innerHTML: string;
  /** Input value string. */
  value: string;
  /** Input type string. */
  type: string;
  /** Inline style map. */
  style: Record<string, string>;

  /**
   * Create a fake element with a tag name.
   * @param tag - Tag name to assign.
   */
  constructor(tag: string) {
    this.tagName = tag;
    this.children = [];
    this.dataset = {};
    this.className = '';
    this.id = '';
    this.textContent = '';
    this.innerHTML = '';
    this.value = '';
    this.type = '';
    this.style = {};
  }
  /**
   * Append a child element.
   * @param child - Child element to append.
   * @returns The appended child.
   */
  appendChild(child: FakeElement) {
    this.children.push(child);
    return child;
  }
  /**
   * Return matching children for a selector.
   * @returns Empty list for this stub.
   */
  querySelectorAll(): FakeElement[] {
    return this.children.flatMap(child => [...(child.dataset['path'] ? [child] : []), ...child.querySelectorAll()]);
  }
  /** No-op event listener registration for the stub. */
  addEventListener(): void {}
}

describe('settings.ts', () => {
  let originalDocument: unknown;
  const globalAny = globalThis as typeof globalThis & { document?: Document };

  beforeEach(() => {
    originalDocument = globalAny.document;
    const mockDocument: Partial<Document> = {
      createElement: (tag: string) => new FakeElement(tag) as unknown as HTMLElement,
      getElementById: () => null
    };
    globalAny.document = mockDocument as Document;
  });

  afterEach(() => {
    globalAny.document = originalDocument as Document;
  });

  it('buildSettingsUI populates the container', () => {
    const container = new FakeElement('div');
    buildSettingsUI(container as unknown as HTMLElement);
    expect(container.children.length).toBeGreaterThan(0);
  });

  /**
   * Recursively collect inputs with data-path attributes.
   * @param root - Root element to traverse.
   * @returns List of matching input elements.
   */
  function collectInputs(root: FakeElement): FakeElement[] {
    const matches: FakeElement[] = [];
    const stack: FakeElement[] = [root];
    while (stack.length > 0) {
      const node = stack.pop()!;
      if (node.dataset && node.dataset['path']) {
        matches.push(node);
      }
      for (const child of node.children) {
        stack.push(child);
      }
    }
    return matches;
  }

  it('buildSettingsUI includes authoritative sensor controls and reset flags', () => {
    const container = new FakeElement('div');
    buildSettingsUI(container as unknown as HTMLElement);
    const inputs = collectInputs(container);
    const paths = inputs.map(input => input.dataset['path']).filter(Boolean);

    expect(paths).toContain('sense.layoutVersion');
    expect(paths).toContain('sense.profile');
    expect(paths).toContain('baselineBots.algorithm');
    expect(paths).toContain('sense.bubbleBins');
    expect(paths).toContain('sense.rNearBase');
    expect(paths).toContain('sense.foodKBase');
    expect(paths).toContain('sense.maxPelletChecks');
    expect(paths).not.toContain('sense.debug');
    expect(paths).not.toContain('observer.focusRecheckSeconds');
    expect(paths).not.toContain('observer.focusSwitchMargin');
    expect(paths).toContain('observer.earlyEndMinSeconds');
    expect(paths).toContain('observer.earlyEndAliveThreshold');
    expect(paths).toContain('observer.overviewPadding');
    expect(paths).toContain('observer.zoomLerpFollow');
    expect(paths).toContain('observer.zoomLerpOverview');
    expect(paths).toContain('observer.overviewExtraWorldMargin');

    const binsInput = inputs.find(input => input.dataset['path'] === 'sense.bubbleBins');
    const collisionCellInput = inputs.find(input => input.dataset['path'] === 'collision.cellSize');

    expect(binsInput?.dataset['requiresReset']).toBe('1');
    expect(collisionCellInput?.dataset['requiresReset']).toBe('1');
  });

  it('stages explicit v4/v2 choices, preserves zero profile codes and omits inactive settings', () => {
    const original = structuredClone(CFG);
    try {
      const panel = new FakeElement('div'); const root = panel as unknown as HTMLElement;
      buildSettingsUI(root); applyValuesToSlidersFromCFG(root);
      const input = (path: string) => panel.querySelectorAll().find(control => control.dataset['path'] === path)!;
      expect(input('sense.layoutVersion').value).toBe('3');
      expect(input('sense.profile').value).toBe('1');
      expect(input('sense.profile').disabled).toBe(true);
      expect(input('baselineBots.behaviorMode').disabled).toBe(true);
      expect(collectSettingsUpdates(root).some(update => update.path.startsWith('sense.ray') || update.path === 'sense.profile')).toBe(false);
      input('sense.layoutVersion').value = '4'; input('sense.profile').value = '0';
      input('baselineBots.algorithm').value = '2';
      refreshSensorSettings(root);
      expect(input('sense.profile').disabled).toBe(false);
      expect(input('baselineBots.behaviorMode').disabled).toBe(false);
      expect(validateSettingDrafts(root)).toBe(true);
      updateCFGFromUI(root);
      expect(CFG.sense.layoutVersion).toBe('v4'); expect(CFG.sense.profile).toBe('basic');
      expect(CFG.brain.inSize).toBe(120);
      expect(collectSettingsUpdates(root)).toEqual(expect.arrayContaining([
        {path: 'sense.layoutVersion', value: 4}, {path: 'sense.profile', value: 0},
        {path: 'baselineBots.algorithm', value: 2}, {path: 'baselineBots.behaviorMode', value: 1},
        {path: 'sense.rayMaximumDue', value: 512}
      ]));
      input('sense.layoutVersion').value = '3';
      expect(validateSettingDrafts(root)).toBe(false);
      expect(input('baselineBots.algorithm').validationMessage).toContain('requires v4');
      updateCFGFromUI(root); expect(CFG.sense.layoutVersion).toBe('v4');
      input('baselineBots.algorithm').value = '1'; refreshSensorSettings(root); updateCFGFromUI(root);
      expect(CFG.brain.inSize).toBe(83);
      const updates = collectSettingsUpdates(root);
      expect(updates.some(update => update.path.startsWith('sense.ray') || update.path === 'sense.profile' ||
        update.path === 'baselineBots.controlDt' || update.path === 'baselineBots.behaviorMode')).toBe(false);
    } finally { Object.assign(CFG, original); }
  });

  it('rejects invalid ray/cadence drafts atomically rather than clamping them', () => {
    const original = structuredClone(CFG);
    try {
      const panel = new FakeElement('div'); const root = panel as unknown as HTMLElement;
      buildSettingsUI(root); applyValuesToSlidersFromCFG(root);
      const input = (path: string) => panel.querySelectorAll().find(control => control.dataset['path'] === path)!;
      input('sense.layoutVersion').value = '4'; input('baselineBots.algorithm').value = '2';
      const reject = (path: string, value: string) => {
        const before = input(path).value; input(path).value = value;
        expect(validateSettingDrafts(root)).toBe(false); updateCFGFromUI(root);
        expect(CFG.sense.layoutVersion).toBe(original.sense.layoutVersion);
        input(path).value = before;
      };
      reject('sense.rayMinimum', '1.5'); reject('sense.rayMinimum', '257');
      reject('sense.rayMinimum', '129'); reject('sense.rayCap', '0');
      reject('baselineBots.controlDt', '0'); reject('baselineBots.behaviorMode', '2');
      expect(validateSettingDrafts(root)).toBe(true);
    } finally { Object.assign(CFG, original); }
  });

  it('applyValuesToSlidersFromCFG updates slider values and labels', () => {
    const original = CFG.boost.minPointsToBoost;
    CFG.boost.minPointsToBoost = 7.5;

    const slider = {
      dataset: { path: 'boost.minPointsToBoost', decimals: '1' },
      value: '0'
    };
    const output = { textContent: '' } as unknown as HTMLElement;
    globalAny.document.getElementById = () => output;

    const root = { querySelectorAll: () => [slider] };
    applyValuesToSlidersFromCFG(root as unknown as HTMLElement);

    expect(slider.value).toBe(String(CFG.boost.minPointsToBoost));
    expect(output.textContent).toBe(fmtNumber(CFG.boost.minPointsToBoost, 1));

    CFG.boost.minPointsToBoost = original;
  });

  it('blocks all draft updates when duration is invalid and accepts unit text', () => {
    const original = CFG.generationSeconds;
    const originalRadius = CFG.worldRadius;
    const input = { dataset: { path: 'generationSeconds', controlType: 'duration' }, type: 'text',
      value: '2m,,3s', setCustomValidity: vi.fn(), setAttribute: vi.fn(), reportValidity: vi.fn() };
    const radius = { dataset: { path: 'worldRadius' }, value: '8000' };
    const root = { querySelectorAll: () => [radius, input] } as unknown as HTMLElement;
    try {
      updateCFGFromUI(root);
      expect(CFG.generationSeconds).toBe(original);
      expect(CFG.worldRadius).toBe(originalRadius);
      expect(input.reportValidity).toHaveBeenCalledOnce();
      input.value = '1h, 2m';
      updateCFGFromUI(root);
      expect(CFG.generationSeconds).toBe(3720);
      expect(CFG.worldRadius).toBe(8000);
    } finally { CFG.generationSeconds = original; CFG.worldRadius = originalRadius; }
  });

  it('renders dropdown labels while retaining explicit integer wire codes and reset metadata', () => {
    const control = createSettingControl({ group: 'Lives', path: 'test.choice', label: 'Memory',
      type: 'select', requiresReset: true, options: [{ value: 0, label: 'Reset' }, { value: 1, label: 'Preserve' }] }) as unknown as FakeElement;
    expect(control.tagName).toBe('select');
    expect(control.dataset).toMatchObject({ path: 'test.choice', controlType: 'select', requiresReset: '1' });
    expect(control.children.map(option => [option.value, option.textContent])).toEqual([['0', 'Reset'], ['1', 'Preserve']]);
    const select = { value: '1', options: [{ value: '0' }, { value: '1' }] };
    expect(readSettingChoice(select as unknown as HTMLSelectElement)).toBe(1);
    for (const value of ['', '2', '0.5', 'NaN']) {
      select.value = value; expect(readSettingChoice(select as unknown as HTMLSelectElement)).toBeNull();
    }
  });

  it('invalid dropdown choices block all reset draft changes and valid zero codes apply', () => {
    const originalRadius = CFG.worldRadius; const originalMode = CFG.brain.gruCrossoverMode;
    const choice = { dataset: { path: 'brain.gruCrossoverMode', controlType: 'select' },
      value: '2', options: [{ value: '0' }, { value: '1' }],
      setCustomValidity: vi.fn(), setAttribute: vi.fn(), reportValidity: vi.fn() };
    const radius = { dataset: { path: 'worldRadius' }, value: '8000' };
    const root = { querySelectorAll: () => [radius, choice] } as unknown as HTMLElement;
    try {
      updateCFGFromUI(root);
      expect(CFG.worldRadius).toBe(originalRadius); expect(CFG.brain.gruCrossoverMode).toBe(originalMode);
      expect(choice.reportValidity).toHaveBeenCalledOnce();
      choice.value = '0'; updateCFGFromUI(root);
      expect(CFG.worldRadius).toBe(8000); expect(CFG.brain.gruCrossoverMode).toBe(0);
      expect(choice.setCustomValidity).toHaveBeenLastCalledWith('');
    } finally { CFG.worldRadius = originalRadius; CFG.brain.gruCrossoverMode = originalMode; }
  });

  it('life preset opts in explicitly while preserving unrelated pending drafts', () => {
    const original = JSON.parse(JSON.stringify(CFG)) as typeof CFG;
    const enabled = { dataset: { path: 'evolvedRespawn.enabled' }, type: 'checkbox', checked: false, value: '0' };
    const overflow = { dataset: { path: 'world.pelletOverflow', controlType: 'select' }, value: '0', options: [{ value: '0', label: 'Fail' }, { value: '1', label: 'Discard' }] };
    const radius = { dataset: { path: 'worldRadius' }, value: '9000' };
    const root = { querySelectorAll: () => [enabled, overflow, radius] } as unknown as HTMLElement;
    try {
      CFG.evolvedRespawn.contractVersion = 0; stageEvolvedRespawnPreset(root);
      expect(CFG.evolvedRespawn.contractVersion).toBe(1); expect(enabled.checked).toBe(true);
      expect(overflow.value).toBe('1'); expect(radius.value).toBe('9000');
    } finally { Object.assign(CFG, original); }
  });

  it('legacy panels preserve the old contract until a life control changes and do not change overflow', () => {
    const original = JSON.parse(JSON.stringify(CFG)) as typeof CFG;
    let listener: (() => void) | undefined;
    const enabled = { dataset: { path: 'evolvedRespawn.enabled', lifeContract: '1', requiresReset: '1' },
      type: 'checkbox', checked: false, value: '0', setCustomValidity: vi.fn(), setAttribute: vi.fn(), reportValidity: vi.fn(),
      addEventListener: (_type: string, callback: () => void) => { listener = callback; } };
    const root = { querySelectorAll: () => [enabled] } as unknown as HTMLElement;
    try {
      CFG.evolvedRespawn.contractVersion = 0; CFG.world.pelletOverflow = 0;
      updateCFGFromUI(root); expect(CFG.evolvedRespawn.contractVersion).toBe(0);
      hookSliderEvents(root, vi.fn()); enabled.checked = true; listener?.(); updateCFGFromUI(root);
      expect(CFG.evolvedRespawn.enabled).toBe(true); expect(CFG.evolvedRespawn.contractVersion).toBe(1);
      expect(CFG.world.pelletOverflow).toBe(0);
      const container = new FakeElement('div'); buildSettingsUI(container as unknown as HTMLElement);
      expect(collectInputs(container).some(input => input.dataset['path'] === 'evolvedRespawn.contractVersion')).toBe(false);
      expect([...EVOLVED_SETTING_PATHS]).toHaveLength(17);
    } finally { Object.assign(CFG, original); }
  });

  it('updateCFGFromUI writes slider values back into CFG', () => {
    const slider = {
      dataset: { path: 'collision.cellSize' },
      value: '123'
    };
    const root = { querySelectorAll: () => [slider] };

    updateCFGFromUI(root as unknown as HTMLElement);

    expect(CFG.collision.cellSize).toBe(123);
  });

  it('applyValuesToSlidersFromCFG handles checkbox inputs', () => {
    const original = CFG.baselineBots.randomizeSeedPerGen;
    CFG.baselineBots.randomizeSeedPerGen = true;
    const checkbox = {
      dataset: { path: 'baselineBots.randomizeSeedPerGen' },
      type: 'checkbox',
      checked: false,
      value: '0'
    };
    const output = { textContent: '' } as unknown as HTMLElement;
    globalAny.document.getElementById = () => output;
    const root = { querySelectorAll: () => [checkbox] };

    applyValuesToSlidersFromCFG(root as unknown as HTMLElement);

    expect(checkbox.checked).toBe(true);
    expect(output.textContent).toBe('On');
    CFG.baselineBots.randomizeSeedPerGen = original;
  });

  it('updateCFGFromUI ignores invalid baseline seed values', () => {
    const original = CFG.baselineBots.seed;
    CFG.baselineBots.seed = 7;
    const input = {
      dataset: { path: 'baselineBots.seed' },
      type: 'number',
      value: '2.5'
    };
    const root = { querySelectorAll: () => [input] };

    updateCFGFromUI(root as unknown as HTMLElement);

    expect(CFG.baselineBots.seed).toBe(7);
    CFG.baselineBots.seed = original;
  });

  it('hookSliderEvents triggers live updates for live sliders', () => {
    const handler = vi.fn();
    let stored: (() => void) | undefined;
    const slider = {
      dataset: { path: 'collision.hitScale', decimals: '2', requiresReset: '0' },
      value: '0.8',
      addEventListener: (evt: string, cb: () => void) => {
        if (evt === 'input') stored = cb;
      }
    };
    const root = { querySelectorAll: () => [slider] };

    hookSliderEvents(root as unknown as HTMLElement, handler);
    stored?.();

    expect(handler).toHaveBeenCalledWith(slider);
  });
});
