import { afterEach, expect, it, vi } from 'vitest';
import { importServerArchive } from './archive_transport.ts';

afterEach(() => {vi.unstubAllGlobals();vi.restoreAllMocks();});

it.each(['error','abort'] as const)('reports an uncertain import after %s without issuing another upload', async kind => {
  const file = new File(['opaque archive bytes'], 'review.slither-save');
  const sent: File[] = [];
  class Upload {
    upload = {};
    onerror?: () => void;
    onabort?: () => void;
    open(): void {}
    setRequestHeader(): void {}
    send(value: File): void {sent.push(value);queueMicrotask(()=>kind==='error'?this.onerror?.():this.onabort?.());}
  }
  vi.stubGlobal('XMLHttpRequest',Upload);
  const alert = vi.fn();vi.stubGlobal('alert',alert);vi.spyOn(console,'error').mockImplementation(()=>{});
  const input = {files:{length:1,item:()=>file},value:'selected'};
  const button = {disabled:false,textContent:'Import and replace world…'};
  await importServerArchive({target:input} as unknown as Event,button as HTMLButtonElement,
    {serverWsUrl:'ws://localhost:5274',serverArchiveImport:true,serverArchiveExport:true,pendingExport:false,isConnected:()=>true});
  expect(sent).toEqual([file]);
  expect(alert).toHaveBeenCalledOnce();
  expect(alert.mock.calls[0]![0]).toMatch(/uncertain|may still complete/u);
  expect(alert.mock.calls[0]![0]).toContain('inspect the current run');
  expect(button).toEqual({disabled:false,textContent:'Import and replace world…'});
  expect(input.value).toBe('');
});
