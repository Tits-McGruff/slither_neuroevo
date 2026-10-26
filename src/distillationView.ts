import type { DistillationDataset } from './protocol/distillation.ts';
import type { ShadowMetricTotals, ShadowSnapshot, ShadowPublication } from './protocol/shadows.ts';
import { DistillationClient } from './net/distillation.ts';

/** One on-demand diagnostic panel, with no render-frame polling or sample/model decoding. */
export class DistillationView {
  /** Connection/run changes invalidate every outstanding presentation. */
  private revision = 0;
  /** Server explicitly advertises the dedicated diagnostic API. */
  private enabled = false;
  /** Structural capture eligibility comes from applied welcome metadata. */
  private captureReady = false;
  /** Current run used to label retained datasets and disallow stale activations. */
  private runId = '';
  /** Serialize user actions across capture and all four shadow slots. */
  private busy = false;
  /** Explicitly selected immutable epoch and its last durable status. */
  private selected: DistillationDataset | undefined;
  /** Last explicitly refreshed native activation publication. */
  private shadows: ShadowPublication | undefined;
  /** Current bounded discovery cursor. */
  private after = '0'.repeat(32);
  /** At most sixteen previous pages, exceeding the admitted 256-dataset inventory. */
  private previous: string[] = [];
  /** Next bounded page cursor, absent at the end. */
  private next: string | undefined;

  /** Bind explicit controls only; construction never starts capture or a trainer. */
  constructor(private readonly client: DistillationClient) {
    this.button('distillationRefresh', () => this.refresh());
    this.button('distillationPrevious', () => this.action(async token => {
      const after = this.previous.at(-1) ?? '0'.repeat(32);
      await this.load(after, token); if (token === this.revision) this.previous.pop();
      return 'Previous datasets loaded.';
    }));
    this.button('distillationNext', () => this.action(async token => {
      if (!this.next) return '';
      const before = this.after; await this.load(this.next, token);
      if (token === this.revision) { this.previous.push(before); this.previous = this.previous.slice(-16); }
      return 'Next datasets loaded.';
    }));
    this.button('distillationStart', () => this.action(async token => {
      const words = this.input('distillationTeachers').value.split(',').map(word => word.trim());
      if (!words.length || words.some(word => !/^[0-9]{1,3}$/u.test(word))) throw new Error('Enter teacher slots separated by commas, for example 0,1,2.');
      const result = await this.client.start(words.map(Number));
      if (token === this.revision) { this.selected = result; this.datasetDetails(); }
      return 'Teacher capture started.';
    }));
    this.button('distillationStop', () => this.action(async token => {
      if (!this.selected) throw new Error('Select an active capture first.');
      const result = await this.client.stop(this.selected.contract.captureEpoch);
      if (token === this.revision) { this.selected = result; this.datasetDetails(); }
      return 'Teacher capture stopped.';
    }));
    this.button('distillationRetry', () => this.action(async token => {
      await this.client.retryShadow(); await this.loadShadows(token); return 'Original shadow command confirmed.';
    }));
    this.button('distillationDismiss', () => {
      this.client.dismissPendingShadow(); this.client.dismissUncertainCapture(); this.message('Pending retry dismissed. An already admitted command can still complete; refresh to see its result.'); this.controls();
    });
    const parent = this.element('distillationShadows');
    for (let slot = 0; slot < 4; slot++) {
      const row = document.createElement('fieldset');
      const legend = document.createElement('legend'); legend.textContent = 'Shadow ' + (slot + 1);
      const label = document.createElement('label'); label.textContent = 'Candidate digest';
      const artifact = document.createElement('input'); artifact.id = 'distillationCandidate' + slot; artifact.type = 'text'; artifact.maxLength = 64;
      label.htmlFor = artifact.id;
      const teacherLabel = document.createElement('label'); teacherLabel.textContent = 'Teacher slot (0-based)';
      const teacher = document.createElement('input'); teacher.id = 'distillationTeacher' + slot; teacher.type = 'number'; teacher.min = '0'; teacher.max = '119'; teacher.step = '1'; teacher.value = '0';
      teacherLabel.htmlFor = teacher.id;
      const actions = document.createElement('div'); actions.className = 'brain-library-actions';
      const activate = this.rowButton('Activate candidate', 'distillationActivate' + slot, () => this.action(async token => {
        if (!this.selected || !this.shadows) throw new Error('Select an active capture and refresh shadow status.');
        if (!/^[0-9]{1,3}$/u.test(teacher.value)) throw new Error('Enter a teacher slot from 0 through 119.');
        await this.client.activate(slot, Number(teacher.value), artifact.value.trim().toLowerCase(), this.selected.contract.captureEpoch, this.shadows);
        await this.loadShadows(token); return 'Shadow candidate activated.';
      }));
      const stop = this.rowButton('Stop shadow', 'distillationStopShadow' + slot, () => this.action(async token => {
        if (!this.shadows) throw new Error('Refresh shadow status first.');
        await this.client.stopShadow(slot, this.shadows); await this.loadShadows(token); return 'Shadow stopped.';
      }));
      const status = document.createElement('div'); status.id = 'distillationShadowStatus' + slot; status.className = 'meta';
      actions.append(activate, stop); row.append(legend, label, artifact, teacherLabel, teacher, actions, status); parent.append(row);
    }
    this.contextChanged(false, false, '');
  }

  /** Welcome/replacement/disconnect invalidates old reads without issuing diagnostic mutations. */
  contextChanged(enabled: boolean, captureReady: boolean, runId: string): void {
    this.revision++; this.enabled = enabled; this.captureReady = captureReady; this.runId = runId;
    this.selected = undefined; this.shadows = undefined; this.after = '0'.repeat(32); this.previous = []; this.next = undefined;
    this.element('distillationDatasets').replaceChildren(); this.element('distillationDataset').textContent = '';
    for (let slot = 0; slot < 4; slot++) this.element('distillationShadowStatus' + slot).textContent = 'Refresh to load shadow status.';
    const tab = document.querySelector<HTMLButtonElement>('[data-tab="tab-distillation"]');
    if (tab) tab.hidden = !enabled;
    this.message(enabled ? (captureReady ? 'Refresh to inspect captures and shadows.' : 'Starting capture requires an admitted v4 sensor profile and a v2 baseline. Refresh can inspect retained datasets and shadows.') : 'Connect to a server supporting distillation.');
    this.controls();
  }

  /** Explicit tab entry or Refresh updates bounded metadata only. */
  refresh(): void {
    this.action(async token => {
      await Promise.all([this.load(this.after, token), this.loadShadows(token)]);
      if (this.selected && token === this.revision) {
        const result = await this.client.manifest(this.selected.contract.captureEpoch);
        if (token === this.revision) { this.selected = result; this.datasetDetails(); }
      }
      return 'Dataset and shadow status refreshed.';
    });
  }

  /** Replace one bounded page after checking connection/run chronology. */
  private async load(after: string, token: number): Promise<void> {
    if (token !== this.revision) return;
    const page = await this.client.datasets(after); if (token !== this.revision) return;
    this.after = after; this.next = page.hasMore ? page.nextCursor : undefined;
    const parent = this.element('distillationDatasets'); parent.replaceChildren();
    for (const dataset of page.datasets) {
      const row = document.createElement('div'); row.className = 'brain-library-item';
      const text = document.createElement('p'); text.className = 'meta'; text.textContent = dataset.captureEpoch + ' · ' + dataset.state + (dataset.runId === this.runId ? ' · current run' : ' · earlier run');
      row.append(text, this.rowButton('Inspect capture', '', () => this.action(async current => {
        const value = await this.client.manifest(dataset.captureEpoch);
        if (current === this.revision) { this.selected = value; this.datasetDetails(); }
        return 'Capture selected.';
      }))); parent.append(row);
    }
    if (!page.datasets.length) parent.textContent = 'No captures on this page.';
  }

  /** Fetch four fixed native slots; old publications never become a new run's controls. */
  private async loadShadows(token: number): Promise<void> {
    if (token !== this.revision) return;
    const status = await this.client.shadows(); if (token !== this.revision) return;
    this.shadows = status;
    for (let slot = 0; slot < 4; slot++) this.shadowDetails(slot, status.slots[slot] ?? null);
  }

  /** Present exact watermarks/drop counts without claiming that every sequence is retained. */
  private datasetDetails(): void {
    const data = this.selected; if (!data) return;
    this.element('distillationDataset').textContent = 'Capture ' + data.contract.captureEpoch + ' · ' + data.state +
      ' · ' + (data.contract.runId === this.runId ? 'current run' : 'earlier run') +
      '\nProduced through ' + data.productionWatermark + ' · stored through ' + data.storageWatermark + ' · resolved through ' + data.resolvedThrough +
      '\nRetained: ' + data.retainedChunks + ' chunks / ' + this.word(data.retainedBytes) + ' bytes' +
      '\nNative drops: ' + this.word(data.nativeDroppedChunks) + ' chunks / ' + this.word(data.nativeDroppedSamples) + ' samples. Storage drops: ' + this.word(data.spoolDroppedChunks) + ' chunks.' +
      '\nTeacher revision ' + data.contract.teacherRevision + ' · ' + (data.contract.teacherMode ? 'stochastic' : 'deterministic') + ' · ' + data.contract.controlIntervalSeconds + ' s · ' + data.contract.inputSize + ' inputs' +
      '\nSensor digest: ' + data.contract.sensorSha256 + (data.error ? '\nCapture error: ' + data.error : '');
  }

  /** Render Rust's fixed aggregate and stratum metrics using text nodes. */
  private shadowDetails(slot: number, snapshot: ShadowSnapshot | null): void {
    const parent = this.element('distillationShadowStatus' + slot); parent.replaceChildren();
    if (!snapshot) { parent.textContent = 'Empty shadow slot.'; return; }
    const identity = document.createElement('p'); identity.textContent = (snapshot.enabled ? 'Active' : 'Stopped') + ' · teacher ' + snapshot.baselineSlot +
      '\nCandidate: ' + snapshot.artifactSha256 + '\nCapture: ' + snapshot.captureEpoch + '\nActivation: ' + snapshot.activationSequence +
      ' · sample step: ' + snapshot.lastSampleStep + (snapshot.error ? '\nError: ' + snapshot.error : '');
    parent.append(identity);
    const groups: [string, ShadowMetricTotals][] = [['All', snapshot.totals], ['Cruising', snapshot.cruising], ['Pre-death', snapshot.preDeath]];
    ['Roam', 'Seek', 'Avoid', 'Boost'].forEach((name, index) => groups.push([name, snapshot.strategies[index]!]));
    ['Small', 'Medium', 'Large'].forEach((name, index) => groups.push([name, snapshot.lifeStages[index]!]));
    ['Hazard', 'Pursuit/escape', 'Boost stratum', 'Transition', 'Saturation', 'Respawn'].forEach((name, index) => groups.push([name, snapshot.strata[index]!]));
    const table = document.createElement('table');
    const header = document.createElement('tr');
    for (const name of ['Group', 'Count', 'Turn MAE', 'Turn MSE', 'Boost disagreement']) { const cell = document.createElement('th'); cell.textContent = name; header.append(cell); }
    table.append(header);
    for (const [name, totals] of groups) {
      const row = document.createElement('tr');
      for (const value of [name, this.word(totals.samples), totals.turnMae.toFixed(4), totals.turnMse.toFixed(4), (totals.boostDisagreementRate * 100).toFixed(2) + '%']) {
        const cell = document.createElement('td'); cell.textContent = value; row.append(cell);
      }
      table.append(row);
    }
    parent.append(table);
  }

  /** Bound one explicit operation and discard responses from an old UI context. */
  private action(action: (token: number) => Promise<string>): void {
    if (!this.enabled || this.busy) return;
    const token = this.revision; this.busy = true; this.controls();
    void action(token).then(message => { if (token === this.revision) this.message(message); })
      .catch(error => { if (token === this.revision) this.message(error instanceof Error ? error.message : String(error), true); })
      .finally(() => { this.busy = false; this.controls(); });
  }

  /** Keep capture/shadow mutations explicit and block new commands while a retry is unresolved. */
  private controls(): void {
    const disabled = !this.enabled || this.busy; const pending = this.client.hasPendingShadow();
    const uncertainCapture = this.client.hasUncertainCapture();
    const active = this.selected?.state === 'active' && this.selected.contract.runId === this.runId;
    for (const id of ['distillationRefresh', 'distillationTeachers', 'distillationPrevious', 'distillationNext']) this.control(id).disabled = disabled;
    this.control('distillationPrevious').disabled ||= !this.previous.length;
    this.control('distillationNext').disabled ||= !this.next;
    this.control('distillationStart').disabled = disabled || pending || uncertainCapture || !this.captureReady || active;
    this.control('distillationStop').disabled = disabled || pending || !active;
    this.control('distillationRetry').hidden = !pending;
    this.control('distillationDismiss').hidden = !pending && !uncertainCapture;
    this.control('distillationRetry').disabled = this.control('distillationDismiss').disabled = disabled;
    this.element('distillationPending').hidden = !pending && !uncertainCapture;
    this.element('distillationPending').textContent = pending
      ? 'The last shadow command may already have completed. Retry checks the original request. Dismissing does not cancel an admitted command.'
      : 'Capture start may already have completed. Refresh captures to locate its epoch; start is never automatically retried. Dismissing does not stop an admitted capture.';
    for (let slot = 0; slot < 4; slot++) {
      this.control('distillationActivate' + slot).disabled = disabled || pending || !active || !this.shadows;
      this.control('distillationStopShadow' + slot).disabled = disabled || pending || !this.shadows?.slots[slot]?.enabled;
      this.control('distillationCandidate' + slot).disabled = disabled;
      this.control('distillationTeacher' + slot).disabled = disabled;
    }
    for (const button of this.element('distillationDatasets').querySelectorAll<HTMLButtonElement>('button')) button.disabled = disabled;
  }

  /** Exact u64 counts remain strings until formatting with lossless BigInt. */
  private word(value: string): string { return BigInt('0x' + value).toLocaleString(); }
  /** Resolve required static or constructed panel elements. */
  private element(id: string): HTMLElement { return document.getElementById(id)!; }
  /** Resolve admitted form controls. */
  private control(id: string): HTMLButtonElement | HTMLInputElement { return this.element(id) as HTMLButtonElement | HTMLInputElement; }
  /** Read a text/numeric input without interpreting imported bytes. */
  private input(id: string): HTMLInputElement { return this.element(id) as HTMLInputElement; }
  /** Wire an explicit static button. */
  private button(id: string, action: () => void): void { this.element(id).addEventListener('click', action); }
  /** Construct one text-only explicit action. */
  private rowButton(label: string, id: string, action: () => void): HTMLButtonElement {
    const button = document.createElement('button'); button.type = 'button'; button.className = 'btn'; button.textContent = label;
    if (id) button.id = id; button.addEventListener('click', action); return button;
  }
  /** Diagnostic strings cannot become HTML. */
  private message(text: string, error = false): void {
    const target = this.element('distillationStatus'); target.textContent = text; target.classList.toggle('error', error);
  }
}
