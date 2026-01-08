import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  buildSettingsUI,
  applyValuesToSlidersFromCFG,
  hookSliderEvents,
  updateCFGFromUI
} from './settings.ts';
import { CFG } from './config.ts';
import { fmtNumber } from './utils.ts';

/** Minimal DOM element stub for settings UI tests. */
class FakeElement {
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
    return [];
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
      dataset: { path: 'collision.cellSize', decimals: '0', requiresReset: '0' },
      value: '80',
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
