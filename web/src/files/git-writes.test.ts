import { expect, it, vi } from 'vitest';
import { WriteQueue } from './git-writes';

/** A write that ends when told to, noting when it starts. */
function deferred(name: string, started: string[]) {
  let end: (failed?: Error) => void = () => {};
  const write = () =>
    new Promise<string>((resolve, reject) => {
      started.push(name);
      end = (failed) => (failed ? reject(failed) : resolve(name));
    });
  return { write, end: (failed?: Error) => end(failed) };
}

it('runs writes one at a time, in order, past one that fails, and is idle once after the last', async () => {
  const idle = vi.fn();
  const queue = new WriteQueue(idle);
  const started: string[] = [];
  const first = deferred('first', started);
  const second = deferred('second', started);
  const ran = [queue.run(first.write), queue.run(second.write)];
  await Promise.resolve();
  expect(started).toEqual(['first']);
  expect(queue.busy).toBe(true);
  first.end(new Error('locked'));
  await expect(ran[0]).rejects.toThrow('locked');
  await vi.waitFor(() => expect(started).toEqual(['first', 'second']));
  expect(idle).not.toHaveBeenCalled();
  second.end();
  await expect(ran[1]).resolves.toBe('second');
  await vi.waitFor(() => expect(idle).toHaveBeenCalledTimes(1));
  expect(queue.busy).toBe(false);
});
