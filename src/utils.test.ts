import { describe, it, expect } from 'vitest';
import { 
  deepClone, 
  clamp, 
  lerp, 
  hypot, 
  angNorm, 
  getByPath, 
  setByPath, 
  fmtNumber,
  hsvToRgb
} from './utils.ts';

describe('utils.ts', () => {
  it('deepClone should create a deep copy', () => {
    const obj = { a: 1, b: { c: 2 } };
    const cloned = deepClone(obj);
    expect(cloned).toEqual(obj);
    expect(cloned).not.toBe(obj);
    expect(cloned.b).not.toBe(obj.b);
  });

  it('clamp should constrain values', () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(-5, 0, 10)).toBe(0);
    expect(clamp(15, 0, 10)).toBe(10);
  });

  it('lerp should interpolate', () => {
    expect(lerp(0, 10, 0.5)).toBe(5);
    expect(lerp(0, 10, 0)).toBe(0);
    expect(lerp(0, 10, 1)).toBe(10);
  });

  it('hypot should calculate hypotenuse', () => {
    expect(hypot(3, 4)).toBe(5);
  });

  it('angNorm should normalize angles to [-PI, PI]', () => {
    expect(angNorm(0)).toBe(0);
    expect(angNorm(Math.PI)).toBe(Math.PI);
    expect(angNorm(-Math.PI)).toBe(-Math.PI);
    expect(angNorm(Math.PI * 1.5)).toBeCloseTo(-Math.PI * 0.5);
    expect(angNorm(-Math.PI * 1.5)).toBeCloseTo(Math.PI * 0.5);
    expect(angNorm(Math.PI * 3)).toBeCloseTo(Math.PI);
  });

  it('getByPath should retrieve nested values', () => {
    const obj = { user: { profile: { name: 'Alice' } } };
    expect(getByPath(obj, 'user.profile.name')).toBe('Alice');
  });

  it('setByPath should set nested values', () => {
    const obj = { user: { profile: { name: 'Alice' } } };
    setByPath(obj, 'user.profile.name', 'Bob');
    expect(obj.user.profile.name).toBe('Bob');
  });

  it('fmtNumber should format numbers correctly', () => {
    expect(fmtNumber(1.2345, 2)).toBe('1.23');
    expect(fmtNumber(1.2345, 0)).toBe('1');
    expect(fmtNumber(1.5, 0)).toBe('2');
  });

  it('hsvToRgb should convert colors', () => {
    // Red (0, 1, 1)
    expect(hsvToRgb(0, 1, 1)).toEqual([255, 0, 0]);
    // Green (1/3, 1, 1)
    expect(hsvToRgb(1/3, 1, 1)).toEqual([0, 255, 0]);
    // Blue (2/3, 1, 1)
    expect(hsvToRgb(2/3, 1, 1)).toEqual([0, 0, 255]);
  });
});
