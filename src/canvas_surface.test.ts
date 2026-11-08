import { describe, expect, it, vi } from 'vitest';
import { fitCanvasToDisplay } from './canvas_surface.ts';

describe('secondary canvas display sizing', () => {
  it('keeps CSS coordinates at high DPI and never reallocates an unchanged backing buffer', () => {
    let width = 360; let height = 300;
    const allocations = vi.fn();
    const canvas = {clientWidth: 390, clientHeight: 300, get width() {return width;}, set width(value) {width = value; allocations();},
      get height() {return height;}, set height(value) {height = value; allocations();}} as HTMLCanvasElement;
    const ctx = {setTransform: vi.fn()} as unknown as CanvasRenderingContext2D;
    expect(fitCanvasToDisplay(canvas, ctx, 2)).toEqual({width:390, height:300, changed:true});
    expect([width, height]).toEqual([780, 600]);
    expect(ctx.setTransform).toHaveBeenLastCalledWith(2, 0, 0, 2, 0, 0);
    for (let frame = 0; frame < 120; frame++) expect(fitCanvasToDisplay(canvas, ctx, 2).changed).toBe(false);
    expect(allocations).toHaveBeenCalledTimes(2);
    Object.assign(canvas, {clientWidth: 280});
    expect(fitCanvasToDisplay(canvas, ctx, 1.5).changed).toBe(true);
    expect([width, height]).toEqual([420, 450]);
    expect(ctx.setTransform).toHaveBeenLastCalledWith(1.5, 0, 0, 1.5, 0, 0);
  });

  it('retains the buffer when its panel is hidden', () => {
    const canvas = {clientWidth:0,clientHeight:0,width:720,height:600} as HTMLCanvasElement;
    const ctx = {setTransform:vi.fn()} as unknown as CanvasRenderingContext2D;
    expect(fitCanvasToDisplay(canvas, ctx, 2).changed).toBe(false);
    expect([canvas.width, canvas.height]).toEqual([720, 600]);
    expect(ctx.setTransform).not.toHaveBeenCalled();
  });
});
