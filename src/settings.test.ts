import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  buildSettingsUI,
  applyValuesToSlidersFromCFG,
  hookSliderEvents,
  updateCFGFromUI
} from './settings.ts';
import { CFG } from './config.ts';
import { fmtNumber } from './utils.ts';

class FakeElement {
  tagName: string;
  children: FakeElement[];
  dataset: Record<string, string>;
  className: string;
  id: string;
  textContent: string;
  innerHTML: string;
  value: string;
  type: string;
  style: Record<string, string>;

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
  appendChild(child: FakeElement) {
    this.children.push(child);
    return child;
  }
  querySelectorAll(): FakeElement[] {
    return [];
  }
  addEventListener(): void {}
}

describe('settings.ts', () => {
  let originalDocument: unknown;
  const globalAny = globalThis as any;

  beforeEach(() => {
    originalDocument = globalAny.document;
    globalAny.document = {
      createElement: (tag: string) => new FakeElement(tag),
      getElementById: () => null
    } as any;
  });

  afterEach(() => {
    globalAny.document = originalDocument;
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
    const output = { textContent: '' };
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
