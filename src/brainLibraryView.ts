import { BrainLibraryClient } from './net/brainLibrary.ts';
import type { SavedBrainRecordV1, SavedBrainSpawnReceiptV1 } from '../server/protocol.ts';

/** Immutable applied facts, separate from graph/settings editor drafts. */
export interface LibraryEnvironment {
  /** Native-advertised graph description; never the editor's legacy graph key. */
  graphSourceJson: string | null;
  /** Exact Rust canonical structural sensor digest. */
  sensorSha256: string | null;
  /** Applied neural cadence. */
  cadence: number;
  /** Current complete run configuration identity. */
  configHash: string;
}

/** Cold user-driven library panel; no work is scheduled by animation frames. */
export class BrainLibraryView {
  /** Discard stale presentation from a previous connection or world replacement. */
  private revision = 0;
  /** Feature admission comes from the connected server's welcome capability. */
  private enabled = false;
  /** One user operation at a time. */
  private busy = false;
  /** Exclusive page cursor; the whole library has at most four 25-record pages. */
  private after: string | null = null;
  /** Small back-navigation stack, never full record caches. */
  private previous: (string | null)[] = [];
  /** Next identity cursor from the last validated page. */
  private next: string | null = null;
  /** Explicit optional archive selection survives page navigation, never a connection change. */
  private readonly archiveSelection = new Set<string>();
  /** An independent capability prevents old library servers silently dropping a selection. */
  private archiveEnabled = false;
  /** One selected full metadata record at most. */
  private selected: SavedBrainRecordV1 | undefined;

  /** Wire ordinary DOM controls; messages and imported labels always use textContent. */
  constructor(
    private readonly client: BrainLibraryClient,
    private readonly environment: () => LibraryEnvironment,
    private readonly spawned: (snakeId: number) => void
  ) {
    this.button('brainLibraryRefresh', () => this.refresh());
    this.button('brainLibraryArchive', () => {
      if (this.busy || !this.enabled || !this.archiveEnabled) return;
      try { this.downloadUrl(this.client.archiveUrl([...this.archiveSelection])); }
      catch (error) { this.message(error instanceof Error ? error.message : String(error), true); }
    });
    this.button('brainLibraryClearArchive', () => {
      this.archiveSelection.clear();
      for (const checkbox of document.querySelectorAll<HTMLInputElement>('[data-brain-archive]')) checkbox.checked = false;
      this.controls();
    });
    this.button('brainLibraryPrevious', () => this.action(async revision => {
      await this.load(this.previous.at(-1) ?? null, revision);
      if (revision === this.revision) this.previous.pop(); return 'Previous page loaded.';
    }));
    this.button('brainLibraryNext', () => this.action(async revision => {
      if (!this.next) return '';
      const previous = this.after; await this.load(this.next, revision);
      if (revision === this.revision) { if (this.previous.length >= 4) this.previous.shift(); this.previous.push(previous); }
      return 'Next page loaded.';
    }));
    this.button('brainLibrarySave', () => this.action(async revision => {
      const result = await this.client.saveCandidate(this.input('brainCandidateId').value.trim().toLowerCase(), this.input('brainLibraryName').value);
      this.previous = []; await this.load(null, revision); return `Saved ${result.name}.`;
    }));
    this.button('brainLibraryImport', () => this.action(async revision => {
      const file = this.input('brainLibraryFile').files?.[0];
      if (!file) throw new Error('Choose a .slither-brain file first.');
      const result = await this.client.importFile(file, this.input('brainLibraryName').value);
      this.previous = []; await this.load(null, revision); return `Imported ${result.name}.`;
    }));
    this.button('brainLibraryRetry', () => this.action(async revision => this.confirm(await this.client.retrySpawn(), revision)));
    this.button('brainLibraryDismiss', () => {
      this.client.dismissPendingSpawn();
      this.message('Pending request dismissed locally. Any already admitted spawn can still complete.'); this.controls();
    });
    this.input('brainLibraryFile').addEventListener('change', () => {
      const file = this.input('brainLibraryFile').files?.[0];
      if (file && !this.input('brainLibraryName').value.trim()) this.input('brainLibraryName').value = file.name.replace(/\.slither-brain$/iu, '');
    });
    this.controls();
  }

  /** Called for welcome/disconnect/replacement; pending retry bodies remain bound to their original process. */
  contextChanged(enabled: boolean, archiveEnabled = false): void {
    this.archiveSelection.clear(); this.archiveEnabled = enabled && archiveEnabled;
    this.revision++; this.enabled = enabled; this.after = null; this.next = null; this.previous = []; this.selected = undefined;
    this.element('brainLibraryList').textContent = '';
    this.element('brainLibraryDetails').textContent = '';
    const tab = document.querySelector<HTMLButtonElement>('[data-tab="tab-library"]');
    if (tab) tab.hidden = !enabled;
    this.message(enabled ? 'Refresh to browse saved brains.' : 'Connect to a server supporting the brain library.');
    this.controls();
  }

  /** Explicit tab entry or Refresh; never poll the catalog each render frame. */
  refresh(): void {
    this.previous = [];
    this.action(async revision => { await this.load(null, revision); return 'Library refreshed.'; });
  }

  /** Applied live settings update compatibility presentation without another model read. */
  environmentChanged(): void { if (this.selected) this.details(this.selected); }

  /** Bounded page replacement with a connection revision guard. */
  private async load(after: string | null, revision: number): Promise<void> {
    if (revision !== this.revision) return;
    const result = await this.client.list(after);
    if (revision !== this.revision) return;
    this.after = after; this.next = result.page.next;
    const list = this.element('brainLibraryList'); list.replaceChildren();
    for (const entry of result.page.entries) {
      const row = document.createElement('div'); row.className = 'brain-library-item';
      const title = document.createElement('strong'); title.textContent = entry.name;
      const meta = document.createElement('div'); meta.className = 'meta';
      meta.textContent = `${entry.artifact.sourceKind} · ${entry.artifact.inputSize} inputs · ${entry.artifact.parameterCount.toLocaleString()} parameters · ${entry.artifact.controlIntervalSeconds.toFixed(4)} s`;
      const actions = document.createElement('div'); actions.className = 'brain-library-actions';
      actions.append(this.rowButton('Details', () => this.action(async token => {
        const record = await this.client.read(entry.brainId);
        if (token === this.revision) { this.selected = record; this.details(record); }
        return 'Saved brain details loaded.';
      })), this.rowButton('Spawn', () => this.action(async token => this.confirm(await this.client.spawn(entry.brainId), token))),
      this.rowButton('Export', () => this.download(entry.brainId)),
      this.rowButton('Delete', () => this.action(async token => {
        await this.client.delete(entry.brainId); this.archiveSelection.delete(entry.brainId); this.previous = []; await this.load(null, token);
        if (token === this.revision && this.selected?.brainId === entry.brainId) { this.selected = undefined; this.element('brainLibraryDetails').textContent = ''; }
        return `Deleted ${entry.name} from the library. Running snakes keep their brains.`;
      })));
      if (this.archiveEnabled) {
        const label = document.createElement('label');
        const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.dataset['brainArchive'] = entry.brainId;
        checkbox.checked = this.archiveSelection.has(entry.brainId); checkbox.disabled = this.busy;
        checkbox.addEventListener('change', () => {
          if (checkbox.checked) this.archiveSelection.add(entry.brainId); else this.archiveSelection.delete(entry.brainId);
          this.controls();
        });
        label.append(checkbox, document.createTextNode(' Include in checkpoint archive')); row.append(label);
      }
      row.append(title, meta, actions); list.append(row);
    }
    if (!result.page.entries.length) list.textContent = 'No saved brains on this page.';
  }

  /** Follow only confirmations that still belong to the current authority. */
  private async confirm(receipt: SavedBrainSpawnReceiptV1, revision: number): Promise<string> {
    const current = revision === this.revision && await this.client.isCurrent(receipt).catch(() => false);
    if (current && revision === this.revision) this.spawned(receipt.snakeId);
    return current ? `Spawned snake #${receipt.snakeId}.` : `Spawn #${receipt.snakeId} was confirmed. Its world is no longer current or could not be checked; camera unchanged.`;
  }

  /** Show structural compatibility and full saved provenance without interpreting model weights. */
  private details(record: SavedBrainRecordV1): void {
    const current = this.environment(); const parent = this.element('brainLibraryDetails'); parent.replaceChildren();
    const heading = document.createElement('h4'); heading.textContent = record.name;
    const compatibility = document.createElement('p'); compatibility.className = 'meta';
    const checks = [current.graphSourceJson === null ? 'Graph: checked by Rust at spawn' : current.graphSourceJson === JSON.stringify(record.graphManifest?.['graphSpec']) ? 'Graph descriptions match; Rust checks the compiled layout at spawn' : 'Graph descriptions differ; Rust checks exact compatibility at spawn',
      current.sensorSha256 === null ? 'Sensors: checked by Rust at spawn' : current.sensorSha256 === record.artifact.sensorSha256 ? 'Structural sensors match' : 'Different structural sensors: matching input width is insufficient',
      current.cadence === record.artifact.controlIntervalSeconds ? 'Neural cadence matches' : `Different neural cadence: saved ${record.artifact.controlIntervalSeconds.toFixed(4)} s; current ${current.cadence.toFixed(4)} s`];
    compatibility.textContent = checks.join('. ') + '.';
    const facts = document.createElement('p'); facts.className = 'meta';
    facts.textContent = `${record.artifact.sourceKind}; ${record.artifact.stateSize} recurrent values; ${record.artifact.lifeStateRule} memory rule. Every spawn starts with zero recurrent state. Source: ${record.brainId}`;
    const provenance = record.provenance.details as Record<string, unknown> | null;
    const teacher = provenance && typeof provenance === 'object' ? provenance['teacherContract'] as Record<string, unknown> | null : null;
    const sourceConfig = teacher && typeof teacher === 'object' ? teacher['configSha256'] : undefined;
    const environment = document.createElement('p'); environment.className = 'meta';
    environment.textContent = typeof sourceConfig === 'string'
      ? sourceConfig === current.configHash.replace(/^sha256:/u, '') ? 'Source configuration matches the current run settings.' : 'Source configuration differs from the current run. Numeric environment changes can affect gameplay; structural compatibility is checked separately.'
      : 'Source configuration was not recorded in this artifact. Review its provenance before evaluating gameplay.';
    const fold = document.createElement('details'); const summary = document.createElement('summary'); summary.textContent = 'Provenance, teacher and validation metrics';
    const pre = document.createElement('pre'); pre.textContent = JSON.stringify(record.provenance, null, 2);
    fold.append(summary, pre); parent.append(heading, compatibility, facts, environment, fold);
  }

  /** Download through a direct native-admitted attachment, with no browser artifact decoding. */
  private download(brainId: string): void {
    if (this.busy || !this.enabled) return;
    try {
      this.downloadUrl(this.client.exportUrl(brainId));
    } catch (error) { this.message(error instanceof Error ? error.message : String(error), true); }
  }

  /** Delegate opaque archive/artifact bytes directly to the browser download manager. */
  private downloadUrl(url: string): void {
    const link = document.createElement('a'); link.href = url; link.download = '';
    link.target = '_blank'; link.rel = 'noopener'; link.hidden = true;
    document.body.append(link); link.click(); link.remove(); this.message('Download requested.');
  }

  /** One operation owns the controls; old connections cannot overwrite current messages or selection. */
  private action(operation: (revision: number) => Promise<string>): void {
    if (this.busy || !this.enabled) return;
    this.busy = true; const revision = this.revision; this.message('Working…'); this.controls();
    void operation(revision).then(message => { if (revision === this.revision) this.message(message); })
      .catch(error => { if (revision === this.revision) this.message(error instanceof Error ? error.message : String(error), true); })
      .finally(() => { this.busy = false; this.controls(); });
  }

  /** Disable every current row/button while a bounded operation is in flight. */
  private controls(): void {
    for (const button of document.querySelectorAll<HTMLButtonElement>('#tab-library button')) button.disabled = this.busy || !this.enabled;
    (this.element('brainLibraryPrevious') as HTMLButtonElement).disabled = this.busy || !this.enabled || !this.previous.length;
    (this.element('brainLibraryNext') as HTMLButtonElement).disabled = this.busy || !this.enabled || !this.next;
    for (const checkbox of document.querySelectorAll<HTMLInputElement>('[data-brain-archive]')) checkbox.disabled = this.busy || !this.enabled;
    this.element('brainLibraryArchiveControls').hidden = !this.archiveEnabled;
    this.element('brainLibraryArchiveCount').textContent = `${this.archiveSelection.size} saved brain${this.archiveSelection.size === 1 ? '' : 's'} selected. Ordinary checkpoint exports exclude the library.`;
    (this.element('brainLibraryArchive') as HTMLButtonElement).disabled = this.busy || !this.archiveEnabled || this.archiveSelection.size === 0;
    (this.element('brainLibraryClearArchive') as HTMLButtonElement).disabled = this.busy || !this.archiveEnabled || this.archiveSelection.size === 0;
    const pending = this.client.hasPendingSpawn();
    this.element('brainLibraryPending').hidden = !pending;
    this.element('brainLibraryRetry').hidden = !pending; this.element('brainLibraryDismiss').hidden = !pending;
  }
  /** Presentation text is never treated as imported HTML. */
  private message(text: string, failed = false): void {
    const status = this.element('brainLibraryStatus'); status.textContent = text; status.classList.toggle('error', failed);
  }
  /** Bind one fixed panel control. */
  private button(id: string, action: () => void): void { this.element(id).addEventListener('click', action); }
  /** Create one action button for the current bounded page. */
  private rowButton(text: string, action: () => void): HTMLButtonElement {
    const button = document.createElement('button'); button.type = 'button'; button.className = 'btn small';
    button.textContent = text; button.addEventListener('click', action); return button;
  }
  /** Required panel elements are installed in index.html. */
  private element(id: string): HTMLElement { return document.getElementById(id)!; }
  /** Typed input access keeps File uploads intact. */
  private input(id: string): HTMLInputElement { return this.element(id) as HTMLInputElement; }
}
