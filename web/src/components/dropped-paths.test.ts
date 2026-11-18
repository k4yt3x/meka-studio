import { afterEach, expect, it, vi } from 'vitest';
import { droppedPaths, dropsAsAddresses } from './dropped-paths';

afterEach(() => {
  vi.unstubAllGlobals();
});

function transfer(...names: string[]) {
  return { files: names.map((name) => new File(['x'], name)) } as unknown as DataTransfer;
}

it('takes the paths the app set for a drop once', async () => {
  const page: Record<string, unknown> = { __MEKA_STUDIO_DROPPED__: ['/home/me/notes', '/a b.txt'] };
  vi.stubGlobal('window', page);
  await expect(droppedPaths(transfer())).resolves.toEqual(['/home/me/notes', '/a b.txt']);
  expect(droppedPaths(transfer())).toBeUndefined();
});

it('asks the app for the paths of the files dropped and takes only its answer', async () => {
  const listeners = new Set<(event: MessageEvent) => void>();
  const posted: { message: string; objects: readonly File[] }[] = [];
  const webview = {
    addEventListener: (_: string, listener: (event: MessageEvent) => void) =>
      listeners.add(listener),
    removeEventListener: (_: string, listener: (event: MessageEvent) => void) =>
      listeners.delete(listener),
    postMessageWithAdditionalObjects: (message: string, objects: readonly File[]) =>
      posted.push({ message, objects }),
  };
  vi.stubGlobal('window', { chrome: { webview } });
  const dropped = transfer('a.png', 'b.txt');
  const found = droppedPaths(dropped);
  expect(posted).toHaveLength(1);
  expect(posted[0]!.objects.map((file) => file.name)).toEqual(['a.png', 'b.txt']);
  const asking = JSON.parse(posted[0]!.message) as { type: string; id: number };
  expect(asking.type).toBe('dropped-paths');
  const id = asking.id;
  const answer = (data: unknown) => {
    for (const listener of [...listeners]) listener({ data } as MessageEvent);
  };
  answer({ type: 'dropped-paths', id: id + 1, paths: ['/elsewhere'] });
  answer({ type: 'dropped-paths', id, paths: 'C:\\a.png' });
  answer({ type: 'dropped-paths', id, paths: ['C:\\a.png', 'C:\\b.txt'] });
  await expect(found).resolves.toEqual(['C:\\a.png', 'C:\\b.txt']);
  expect(listeners.size).toBe(0);
});

it('takes a drag of addresses alone for files on Linux only', () => {
  const addresses = { types: ['text/uri-list'] } as unknown as DataTransfer;
  vi.stubGlobal('navigator', { platform: 'Linux x86_64' });
  expect(dropsAsAddresses(addresses)).toBe(true);
  expect(dropsAsAddresses({ types: ['text/plain'] } as unknown as DataTransfer)).toBe(false);
  vi.stubGlobal('navigator', { platform: 'Win32' });
  expect(dropsAsAddresses(addresses)).toBe(false);
  vi.stubGlobal('navigator', { platform: 'MacIntel' });
  expect(dropsAsAddresses(addresses)).toBe(false);
});

it('tells nothing where the app cannot', () => {
  vi.stubGlobal('window', {});
  expect(droppedPaths(transfer('a.png'))).toBeUndefined();
  vi.stubGlobal('window', { __MEKA_STUDIO_DROPPED__: [] });
  expect(droppedPaths(transfer())).toBeUndefined();
});
