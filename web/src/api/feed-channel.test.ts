import { expect, it, vi } from 'vitest';
import { ApiError, ConnectionError } from './client';
import { FeedChannel } from './feed-channel';

/**
 * Meka Studio's side of the stream: what it says, and the follows it was asked for, each answered
 * at once unless `answers` holds how to answer it first.
 */
function server(...answers: (() => Promise<void>)[]) {
  const encoder = new TextEncoder();
  const streams: ReadableStreamDefaultController<Uint8Array>[] = [];
  const sent: [string, string, unknown][] = [];
  const gateway = {
    response: vi.fn(async () => {
      let controller!: ReadableStreamDefaultController<Uint8Array>;
      const body = new ReadableStream<Uint8Array>({
        start(made) {
          controller = made;
        },
      });
      streams.push(controller);
      return new Response(body);
    }),
    send: vi.fn(async (method: string, path: string, body?: unknown) => {
      sent.push([method, path, body]);
      if (method === 'PUT') await answers.shift()?.();
    }),
  };
  const say = (text: string, stream = streams.length - 1) =>
    streams[stream]!.enqueue(encoder.encode(text));
  return { gateway, sent, say, streams };
}

async function text(response: Response) {
  return new Response(response.body).text();
}

it('hands each feed on as its own event stream, as it came', async () => {
  const s = server();
  const channel = new FeedChannel(s.gateway);
  const following = channel.follow(
    { kind: 'feed', path: '/v1/sessions/a/feed', cursor: '4' },
    new AbortController().signal,
  );
  s.say('data: ! opened one\n\n');
  await vi.waitFor(() => expect(s.sent).toHaveLength(1));
  expect(s.sent[0]).toEqual([
    'PUT',
    '/api/feeds/one/f1',
    { kind: 'feed', path: '/v1/sessions/a/feed', cursor: '4' },
  ]);
  s.say('data: ! started f1\n\ndata: f1 id: 5\ndata: f1 event: turn.started\n');
  s.say('data: f1 data: {"a": "b c"}\ndata: f1 \n\ndata: f9 not this one\n\n');
  const response = await following;
  // What came before the feed ended, read with it, still reaches it.
  s.say('data: f1 : bye\ndata: ! ended f1\n\n');
  expect(await text(response)).toBe('id: 5\nevent: turn.started\ndata: {"a": "b c"}\n\n: bye\n');
});

it('fails a feed Meka refuses with its problem, and lets one go when asked', async () => {
  const s = server();
  const channel = new FeedChannel(s.gateway);
  const refused = channel.follow({ kind: 'feed', path: '/v1/feed' }, new AbortController().signal);
  s.say('data: ! opened one\n\n');
  await vi.waitFor(() => expect(s.sent).toHaveLength(1));
  s.say(
    'data: ! failed f1 409 3 {"type": "https://meka.bot/errors/session-not-loaded", "status": 409}\n\n',
  );
  const error = await refused.catch((failure: unknown) => failure);
  expect(error).toBeInstanceOf(ApiError);
  expect((error as ApiError).status).toBe(409);
  expect((error as ApiError).retryAfter).toBe(3000);
  expect((error as ApiError).problem.type).toContain('session-not-loaded');
  const stop = new AbortController();
  const following = channel.follow({ kind: 'folders', paths: ['/w'] }, stop.signal);
  await vi.waitFor(() => expect(s.sent).toHaveLength(2));
  s.say('data: ! started f2\n\n');
  const response = await following;
  stop.abort();
  await expect(text(response)).rejects.toBeDefined();
  await vi.waitFor(() => expect(s.sent.at(-1)).toEqual(['DELETE', '/api/feeds/one/f2', undefined]));
  // One stream carried both.
  expect(s.gateway.response).toHaveBeenCalledTimes(1);
});

it('lets a feed go only once Meka Studio has answered following it', async () => {
  let answer!: () => void;
  const s = server(() => new Promise<void>((resolve) => (answer = resolve)));
  const channel = new FeedChannel(s.gateway);
  const stop = new AbortController();
  const following = channel.follow({ kind: 'feed', path: '/v1/feed' }, stop.signal);
  s.say('data: ! opened one\n\n');
  await vi.waitFor(() => expect(s.sent).toHaveLength(1));
  stop.abort();
  await expect(following).rejects.toBeDefined();
  await new Promise((resolve) => setTimeout(resolve, 10));
  expect(s.sent).toHaveLength(1);
  answer();
  await vi.waitFor(() => expect(s.sent.at(-1)).toEqual(['DELETE', '/api/feeds/one/f1', undefined]));
});

it('ends every feed with the stream, cleanly unless it broke, and opens another for the next', async () => {
  const s = server();
  const channel = new FeedChannel(s.gateway);
  const first = channel.follow({ kind: 'feed', path: '/v1/feed' }, new AbortController().signal);
  s.say('data: ! opened one\n\n');
  await vi.waitFor(() => expect(s.sent).toHaveLength(1));
  s.say('data: ! started f1\n\ndata: f1 : hi\n');
  const ended = await first;
  s.streams[0]!.close();
  expect(await text(ended)).toBe(': hi\n');
  const next = channel.follow({ kind: 'feed', path: '/v1/feed' }, new AbortController().signal);
  await vi.waitFor(() => expect(s.gateway.response).toHaveBeenCalledTimes(2));
  s.say('data: ! opened two\n\n');
  await vi.waitFor(() => expect(s.sent).toHaveLength(2));
  expect(s.sent[1]![1]).toBe('/api/feeds/two/f2');
  s.say('data: ! started f2\n\n');
  const broken = await next;
  s.streams[1]!.error(new TypeError('network'));
  await expect(text(broken)).rejects.toBeInstanceOf(ConnectionError);
});

it('opens another stream as the device’s data saving changes, the feeds moving to it', async () => {
  const s = server();
  let saving = false;
  const channel = new FeedChannel(s.gateway, () =>
    saving ? { 'X-Meka-Studio-Data-Saver': '1' } : {},
  );
  const first = channel.follow({ kind: 'feed', path: '/v1/feed' }, new AbortController().signal);
  s.say('data: ! opened one\n\n');
  await vi.waitFor(() => expect(s.sent).toHaveLength(1));
  s.say('data: ! started f1\n\n');
  const started = await first;
  const waiting = channel.follow(
    { kind: 'feed', path: '/v1/sessions/a/feed' },
    new AbortController().signal,
  );
  await vi.waitFor(() => expect(s.sent).toHaveLength(2));
  saving = true;
  const last = channel.follow({ kind: 'feed', path: '/v1/feed' }, new AbortController().signal);
  await vi.waitFor(() => expect(s.gateway.response).toHaveBeenCalledTimes(2));
  expect(s.gateway.response).toHaveBeenLastCalledWith('GET', '/api/feeds', {
    signal: expect.any(AbortSignal),
    headers: { Accept: 'text/event-stream', 'X-Meka-Studio-Data-Saver': '1' },
  });
  // The feed started on the old stream ends, to be followed again; the one waiting moves.
  expect(await text(started)).toBe('');
  s.say('data: ! opened two\n\n');
  await vi.waitFor(() => expect(s.sent).toHaveLength(4));
  expect(s.sent.slice(2).map(([, path, body]) => [path, body])).toEqual([
    ['/api/feeds/two/f3', { kind: 'feed', path: '/v1/sessions/a/feed' }],
    ['/api/feeds/two/f4', { kind: 'feed', path: '/v1/feed' }],
  ]);
  s.say('data: ! started f3\n\ndata: ! started f4\n\n');
  await Promise.all([waiting, last]);
  expect(s.gateway.response).toHaveBeenCalledTimes(2);
});

it('moves a feed to another stream when Meka Studio has ended the one it was asked on', async () => {
  const gone = () => Promise.reject(new ApiError(404, { detail: 'no such stream is open' }));
  const s = server(gone, gone);
  const channel = new FeedChannel(s.gateway);
  const following = channel.follow(
    { kind: 'feed', path: '/v1/feed' },
    new AbortController().signal,
  );
  s.say('data: ! opened one\n\n');
  await vi.waitFor(() => expect(s.gateway.response).toHaveBeenCalledTimes(2));
  s.say('data: ! opened two\n\n');
  // Gone again, the stream is not tried a third time for it.
  const error = await following.catch((failure: unknown) => failure);
  expect(error).toBeInstanceOf(ConnectionError);
  expect(s.sent.map(([method, path]) => `${method} ${path}`)).toEqual([
    'PUT /api/feeds/one/f1',
    'PUT /api/feeds/two/f2',
  ]);
});
