import { AdvancedCharts } from './chartUtils.ts';
import { fitCanvasToDisplay } from './canvas_surface.ts';
import type { FrameStats } from './protocol/messages.ts';

/** Only bounded sidebar presentation; history and runtime state remain owned by main. */
export class StatsView {
  private chartKey = '';
  private logKey = '';
  private infoKey = '';

  /** Refresh a visible panel at sidebar cadence, redrawing history only when it changes. */
  update(stats: FrameStats, history: Parameters<typeof AdvancedCharts.renderAverageFitness>[1], logs: {time: number; action: string; snakeId: number; result: string}[], view: string): void {
    const canvas = document.getElementById('statsCanvas') as HTMLCanvasElement;
    const ctx = canvas.getContext('2d')!;
    const size = fitCanvasToDisplay(canvas, ctx);
    const chartKey = JSON.stringify([view, history, size.width, size.height, canvas.width, canvas.height]);
    if (size.width && size.height && (chartKey !== this.chartKey || size.changed)) {
      this.chartKey = chartKey;
      ctx.clearRect(0, 0, size.width, size.height);
      const render = view === 'diversity' ? AdvancedCharts.renderSpeciesDiversity : view === 'complexity' ? AdvancedCharts.renderNetworkComplexity : AdvancedCharts.renderAverageFitness;
      render(ctx, history, size.width, size.height);
      const summary = document.getElementById('chartSummary');
      const last = history.at(-1);
      if (summary) summary.textContent = last
        ? `Generation ${last.gen}: ${view === 'diversity' ? `${last.speciesCount ?? 0} species; largest ${last.topSpeciesSize ?? 0} individuals` : view === 'complexity' ? `mean |weight| ${(last.avgWeight ?? 0).toFixed(3)}; variance ${(last.weightVariance ?? 0).toFixed(3)}` : `fitness min ${(last.minFitness ?? 0).toFixed(1)}, mean ${(last.avgFitness ?? 0).toFixed(1)}, max ${(last.maxFitness ?? 0).toFixed(1)}`}. X axis: generation.`
        : 'No completed generations yet. History appears after the first generation finishes.';
    }
    const info = document.getElementById('statsInfo');
    const life = stats.evolved;
    const facts: [string, string | number][] = [
      ['Generation', stats.gen], ['Alive (population)', stats.alive], ['Alive (total)', stats.aliveTotal ?? stats.alive],
      ['Baseline bots', `${stats.baselineBotsAlive ?? 0}/${stats.baselineBotsTotal ?? 0}`], ['Server step rate', `${Math.round(stats.fps)} Hz`]
    ];
    if (life) facts.push(['Waiting / pending', `${life.waiting} / ${life.pendingPlacement}`], ['Exhausted individuals', life.exhausted],
      ['Deaths / respawns', `${BigInt('0x' + life.deaths)} / ${BigInt('0x' + life.respawns)}`], ['Individual waiting total', `${life.pendingSeconds.toFixed(1)} sim s`],
      ['Failed placements', `${BigInt('0x' + life.failedPlacements)}`], ['Placement step', `${BigInt('0x' + life.placement.step)}`],
      ['Attempts / deferred', `${life.placement.attempted} / ${life.placement.deferred}`], ['Candidates', `${life.placement.candidates}/2048`], ['Geometry checks', `${life.placement.geometryChecks}/250000`]);
    const infoKey = JSON.stringify(facts);
    if (info && infoKey !== this.infoKey) {
      this.infoKey = infoKey;
      info.innerHTML = facts.map(([label, value]) => `<div class="stat-box"><span class="label">${label}</span><span class="val">${value}</span></div>`).join('');
    }
    const log = document.getElementById('godModeLog');
    const recent = logs.slice(-10).reverse();
    const logKey = JSON.stringify(recent);
    if (log && logKey !== this.logKey) {
      this.logKey = logKey;
      log.replaceChildren();
      for (const entry of recent) {
        const line = document.createElement('div'); line.className = 'log-entry';
        line.textContent = `[${new Date(entry.time).toLocaleTimeString()}] ${entry.action.toUpperCase()} snake #${entry.snakeId} — ${entry.result}`;
        log.appendChild(line);
      }
      if (!recent.length) log.textContent = 'No God Mode interactions yet.';
    }
  }
}
