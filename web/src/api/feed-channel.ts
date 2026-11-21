import { ApiError, ConnectionError, type Schema } from './client';
import type { GatewayClient } from './gateway';

/** What a page asks its stream of feeds to follow: one of Meka's feeds, or changes to folders. */
export type Follow =
  | { kind: 'feed'; path: string; cursor?: string; attend?: boolean; focused?: boolean }
  | { kind: 'folders'; paths: string[] };

/**
 * How a feed, or the stream carrying it, ends: with the error it failed with; `ended`, as Meka
 * Studio ended it; or `replaced`, by a stream the page opened in its place.
 */
type Ending = Error | 'ended' | 'replaced';

/** A feed followed: what its lines become, and how it goes. */
interface Following {
  started: () => void;
  /** Lines of the feed, each ended. */
  lines: (text: string) => void;
  end: (how: Ending) => void;
}

/**
 * The page's one stream of feeds from Meka Studio, which follows Meka's feeds and folders for it.
 * Over HTTP/1.1, which the desktop app always speaks, a browser holds about six connections to a
 * server, and a stream for each feed, as for an agent's many sub-agents, would leave none for the
 * page's other requests. Each feed followed comes back as a response of its own, whose body is the
 * feed's event stream as it came. The stream ending ends every feed on it, cleanly unless it broke,
 * and following again opens another. A change of the device's data saving opens another as the
 * next feed is followed, ending the old stream's feeds as cleanly; those not started yet follow on
 * it.
 */
export class FeedChannel {
  private opened: Opened | undefined;
  private next = 0;

  constructor(
    private gateway: {
      response: GatewayClient['response'];
      send: (method: 'PUT' | 'DELETE', path: string, body?: unknown) => Promise<unknown>;
    },
    private headers: () => Record<string, string> = () => ({}),
  ) {}

  /**
   * Follows `follow` until `signal` aborts: once it starts, a response whose body is its event
   * stream. Meka refusing it fails with its problem, as a request would.
   */
  follow(follow: Follow, signal: AbortSignal): Promise<Response> {
    return this.attempt(follow, signal, false);
  }

  /** Follows `follow` on the stream open; `moved` once its first stream was replaced. */
  private async attempt(follow: Follow, signal: AbortSignal, moved: boolean): Promise<Response> {
    signal.throwIfAborted();
    const opened = this.open();
    const key = `f${++this.next}`;
    const encoder = new TextEncoder();
    let body!: ReadableStreamDefaultController<Uint8Array>;
    let started = false;
    let over = false;
    let path = '';
    /** Whether Meka Studio took the follow, once it was asked. */
    let asked: Promise<boolean> | undefined;
    return new Promise<Response>((resolve, reject) => {
      const finish = () => {
        if (over) return false;
        over = true;
        opened.follows.delete(key);
        signal.removeEventListener('abort', aborted);
        return true;
      };
      // Let go of by the page. Meka Studio is told only once it has answered the follow, which
      // a request sent beside it could otherwise overtake, leaving the feed followed.
      const stop = (reason: unknown) => {
        if (!finish()) return;
        void asked
          ?.then((taken) => (taken ? this.gateway.send('DELETE', path) : undefined))
          .catch(() => {});
        quietly(() => body.error(reason));
        if (!started) reject(reason);
      };
      const aborted = () => stop(signal.reason);
      const stream = new ReadableStream<Uint8Array>({
        start: (controller) => {
          body = controller;
        },
        cancel: (reason) => stop(reason),
      });
      opened.follows.set(key, {
        started: () => {
          started = true;
          resolve(new Response(stream, { headers: { 'Content-Type': 'text/event-stream' } }));
        },
        lines: (text) => quietly(() => body.enqueue(encoder.encode(text))),
        end: (how) => {
          if (!finish()) return;
          if (how instanceof Error) {
            if (started) quietly(() => body.error(how));
            else reject(how);
          } else if (started) quietly(() => body.close());
          // Replaced before it started, it follows on the stream in its place, once.
          else if (how === 'replaced' && !moved) resolve(this.attempt(follow, signal, true));
          else reject(new ConnectionError());
        },
      });
      signal.addEventListener('abort', aborted, { once: true });
      opened.name.then(
        (name) => {
          if (over) return;
          path = `/api/feeds/${encodeURIComponent(name)}/${key}`;
          asked = this.gateway.send('PUT', path, follow).then(
            () => true,
            (error: unknown) => {
              // Meka Studio no longer has the stream, which ended there before it reached here.
              if (error instanceof ApiError && error.status === 404) this.end(opened, 'replaced');
              else
                opened.follows
                  .get(key)
                  ?.end(error instanceof Error ? error : new ConnectionError(error));
              return false;
            },
          );
        },
        // The stream's end has ended this follow too.
        () => {},
      );
    });
  }

  /** The stream open, opening it if there is none, or the one open is for other data saving. */
  private open(): Opened {
    const headers = this.headers();
    const sent = JSON.stringify(headers);
    const current = this.opened;
    if (current?.sent === sent) return current;
    let named!: (name: string) => void;
    let refused!: (error: Error) => void;
    const name = new Promise<string>((resolve, reject) => {
      named = resolve;
      refused = reject;
    });
    // A stream that failed to open is no promise left waiting.
    name.catch(() => {});
    const opened: Opened = {
      name,
      named,
      refused,
      follows: new Map(),
      held: new Map(),
      abort: new AbortController(),
      sent,
      over: false,
    };
    // Open before the old one ends, so the feeds that move from it find this one.
    this.opened = opened;
    void this.read(opened, headers);
    if (current) this.end(current, 'replaced');
    return opened;
  }

  /** Ends `opened` and every feed it carries, as `how` says. */
  private end(opened: Opened, how: Ending) {
    if (opened.over) return;
    opened.over = true;
    if (this.opened === opened) this.opened = undefined;
    opened.abort.abort();
    opened.refused(how instanceof Error ? how : new ConnectionError());
    for (const following of [...opened.follows.values()]) following.end(how);
  }

  /** Reads `opened` until it ends, and ends it with how it did. */
  private async read(opened: Opened, headers: Record<string, string>) {
    let response: Response;
    try {
      response = await this.gateway.response('GET', '/api/feeds', {
        signal: opened.abort.signal,
        headers: { Accept: 'text/event-stream', ...headers },
      });
    } catch (error) {
      this.end(opened, error instanceof Error ? error : new ConnectionError(error));
      return;
    }
    const reader = response.body?.pipeThrough(new TextDecoderStream()).getReader();
    if (!reader) {
      this.end(opened, new ConnectionError());
      return;
    }
    // The line begun, in the pieces it came in: only what was just read can end it, so a long line
    // is never searched again from its start.
    let begun: string[] = [];
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        let start = 0;
        let end: number;
        while ((end = value.indexOf('\n', start)) >= 0) {
          begun.push(value.slice(start, end));
          this.line(opened, begun.join('').replace(/\r$/, ''));
          begun = [];
          start = end + 1;
        }
        if (start < value.length) begun.push(value.slice(start));
        this.hand(opened);
      }
    } catch (error) {
      this.end(opened, new ConnectionError(error));
      return;
    }
    this.end(opened, 'ended');
  }

  /**
   * Takes in one line of `opened`: a feed's own, held to hand on with the others read with it, or
   * what the stream says of one, after the feeds have what came before it.
   */
  private line(opened: Opened, line: string) {
    if (!line.startsWith('data: ')) return;
    const rest = line.slice(6);
    const space = rest.indexOf(' ');
    const key = space < 0 ? rest : rest.slice(0, space);
    const text = space < 0 ? '' : rest.slice(space + 1);
    if (key !== '!') {
      const following = opened.follows.get(key);
      if (!following) return;
      const held = opened.held.get(following);
      if (held) held.push(text);
      else opened.held.set(following, [text]);
      return;
    }
    this.hand(opened);
    const [said = '', of = '', status = '', retry = '-'] = text.split(' ', 4);
    if (said === 'opened') opened.named(of);
    else if (said === 'started') opened.follows.get(of)?.started();
    else if (said === 'ended') opened.follows.get(of)?.end('ended');
    else if (said === 'failed') {
      // `failed <key> <status> <retry-after seconds, or -> <problem>`
      let problem: Partial<Schema['ProblemDetail']> = {};
      try {
        const value: unknown = JSON.parse(text.split(' ').slice(4).join(' '));
        if (value && typeof value === 'object') problem = value as Partial<Schema['ProblemDetail']>;
      } catch {
        // Said without one.
      }
      const seconds = Number(retry);
      opened.follows
        .get(of)
        ?.end(
          new ApiError(
            Number(status) || 502,
            problem,
            retry !== '-' && Number.isFinite(seconds) ? seconds * 1000 : undefined,
          ),
        );
    }
  }

  /** Hands each feed on `opened` the lines held for it, together. */
  private hand(opened: Opened) {
    for (const [following, texts] of opened.held) following.lines(texts.join('\n') + '\n');
    opened.held.clear();
  }
}

/** Runs `step` on a body that may be closed already, as by its reader, and takes no more. */
function quietly(step: () => void) {
  try {
    step();
  } catch {
    // Closed already.
  }
}

/** A stream open, or opening: its name once it says it, the feeds on it, and how it was asked. */
interface Opened {
  name: Promise<string>;
  named: (name: string) => void;
  refused: (error: Error) => void;
  follows: Map<string, Following>;
  /** Each feed's lines read and not yet handed on. */
  held: Map<Following, string[]>;
  abort: AbortController;
  /** The headers it was opened with, as the device saves data or not. */
  sent: string;
  /** Whether it ended. */
  over: boolean;
}
