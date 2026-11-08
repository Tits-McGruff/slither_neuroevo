/** Graph preset HTTP operations and presentation with an explicit browser owner port. */
import type { GraphSpec } from './brains/graph/schema.ts';
import { validateGraph } from './brains/graph/validate.ts';

/** The existing draft/session access required by preset operations. */
export interface GraphPresetPort {
  readonly graphPresetList: HTMLElement | null;
  readonly graphPresetName: HTMLInputElement | null;
  readonly resolveBaseUrl: () => string | null;
  readonly setGraphSpecStatus: (text: string, isError?: boolean) => void;
  readonly setGraphDraft: (spec: GraphSpec, note?: string) => void;
  readonly getDraftGraphSpec: () => GraphSpec;
}


/** Saved graph preset summary from the server. */
type SavedGraphPreset = { id: number; name: string; createdAt: number };

/** Loaded graph preset including the graph spec. */
type LoadedGraphPreset = SavedGraphPreset & { spec: GraphSpec };

function renderSavedPresetList(presets: SavedGraphPreset[], ports: GraphPresetPort): void {
  const graphPresetList = ports.graphPresetList;
  if (!graphPresetList) return;
  graphPresetList.innerHTML = '';
  if (!presets.length) {
    const empty = document.createElement('div');
    empty.className = 'meta';
    empty.textContent = 'No saved presets yet.';
    graphPresetList.appendChild(empty);
    return;
  }
  presets.forEach(preset => {
    const row = document.createElement('div');
    row.className = 'graph-preset-item';
    const name = document.createElement('div');
    name.className = 'graph-preset-name';
    name.textContent = preset.name;
    const load = document.createElement('button');
    load.className = 'btn small';
    load.textContent = 'Load';
    load.addEventListener('click', () => {
      void loadPresetById(preset.id, ports);
    });
    row.appendChild(name);
    row.appendChild(load);
    graphPresetList.appendChild(row);
  });
}

async function loadPresetById(presetId: number, ports: GraphPresetPort): Promise<void> {
  const baseUrl = ports.resolveBaseUrl();
  if (!baseUrl || typeof fetch === 'undefined') {
    ports.setGraphSpecStatus('Server not available for loading presets.', true);
    return;
  }
  try {
    const response = await fetch(`${baseUrl}/api/graph-presets/${presetId}`);
    const data = (await response.json()) as { ok?: boolean; preset?: LoadedGraphPreset; message?: string };
    if (!response.ok || !data.ok || !data.preset) {
      ports.setGraphSpecStatus(data.message || 'Failed to load preset.', true);
      return;
    }
    const result = validateGraph(data.preset.spec);
    if (!result.ok) {
      ports.setGraphSpecStatus(`Preset graph invalid: ${result.reason}`, true);
      return;
    }
    ports.setGraphDraft(data.preset.spec, `Loaded preset "${data.preset.name}". Stage graph to use it.`);
  } catch (err) {
    console.warn('[graph-presets] load failed', err);
    ports.setGraphSpecStatus('Failed to load preset.', true);
  }
}

/** Refresh the bounded preset list using the current session URL. */
export async function refreshSavedPresets(ports: GraphPresetPort): Promise<void> {
  const graphPresetList = ports.graphPresetList;
  if (!graphPresetList) return;
  const baseUrl = ports.resolveBaseUrl();
  if (!baseUrl) {
    renderSavedPresetList([], ports);
    return;
  }
  if (typeof fetch === 'undefined') {
    ports.setGraphSpecStatus('Fetch not available for saved presets.', true);
    return;
  }
  try {
    const response = await fetch(`${baseUrl}/api/graph-presets?limit=50`);
    const data = (await response.json()) as { ok?: boolean; presets?: SavedGraphPreset[]; message?: string };
    if (!response.ok || !data.ok) {
      ports.setGraphSpecStatus(data.message || 'Failed to load saved presets.', true);
      renderSavedPresetList([], ports);
      return;
    }
    renderSavedPresetList(data.presets ?? [], ports);
  } catch (err) {
    console.warn('[graph-presets] load failed', err);
    ports.setGraphSpecStatus('Failed to load saved presets.', true);
    renderSavedPresetList([], ports);
  }
}

/** Save the current draft without acquiring graph or connection ownership. */
export async function saveGraphPreset(ports: GraphPresetPort): Promise<void> {
  const graphPresetName = ports.graphPresetName;
    const name = graphPresetName?.value.trim() ?? '';
    if (!name) {
      ports.setGraphSpecStatus('Preset name is required.', true);
      return;
    }
    const baseUrl = ports.resolveBaseUrl();
    if (!baseUrl || typeof fetch === 'undefined') {
      ports.setGraphSpecStatus('Server not available for saving presets.', true);
      return;
    }
    const spec = ports.getDraftGraphSpec();
    try {
      const response = await fetch(`${baseUrl}/api/graph-presets`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name, spec })
      });
      const data = (await response.json()) as { ok?: boolean; presetId?: number; message?: string };
      if (!response.ok || !data.ok) {
        ports.setGraphSpecStatus(data.message || 'Failed to save preset.', true);
        return;
      }
      ports.setGraphSpecStatus(`Saved preset "${name}".`);
      if (graphPresetName) graphPresetName.value = '';
      await refreshSavedPresets(ports);
    } catch (err) {
      console.warn('[graph-presets] save failed', err);
      ports.setGraphSpecStatus('Failed to save preset.', true);
    }
  }
