import { describe, expect, it } from 'vitest';
import { screenToSvg } from './graph_diagram.ts';

describe('graph pointer coordinates', () => {
  it.each([[0.4, 25, 160], [2, 250, 20]])('inverts scale %s with letterboxing at %s,%s', (scale, left, top) => {
    const svg = {getScreenCTM: () => ({inverse: () => ({scale, left, top})}),
      createSVGPoint: () => ({x:0,y:0,matrixTransform(matrix: {scale:number;left:number;top:number}) {
        return {x:(this.x-matrix.left)/matrix.scale,y:(this.y-matrix.top)/matrix.scale};
      }})} as unknown as SVGSVGElement;
    const point = screenToSvg(svg, {clientX:left+scale*140,clientY:top+scale*44});
    expect(point?.x).toBeCloseTo(140); expect(point?.y).toBeCloseTo(44);
  });
  it('ignores detached or singular SVG surfaces', () => {
    expect(screenToSvg(null, {clientX:0,clientY:0})).toBeNull();
    expect(screenToSvg({getScreenCTM:()=>null} as unknown as SVGSVGElement, {clientX:0,clientY:0})).toBeNull();
    expect(screenToSvg({getScreenCTM:()=>({inverse:()=>{throw new Error('singular');}}),createSVGPoint:()=>({})} as unknown as SVGSVGElement, {clientX:0,clientY:0})).toBeNull();
  });
});
