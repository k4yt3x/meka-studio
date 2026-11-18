/**
 * Runs writes one at a time, in the order asked, each whether or not the one before it failed, so
 * git never finds its index locked by one of them; once the last asked for so far is done, tells
 * `onIdle`, which a write asked for meanwhile waits on.
 */
export class WriteQueue {
  private tail: Promise<void> = Promise.resolve();
  private waiting = 0;

  constructor(private onIdle: () => Promise<void> | void) {}

  /** Whether a write is asked for and not done. */
  get busy(): boolean {
    return this.waiting > 0;
  }

  /** Runs `write` after those asked for before it; what it gives, or why it failed. */
  run<T>(write: () => Promise<T>): Promise<T> {
    this.waiting++;
    const done = this.tail.then(write);
    this.tail = done
      .then(
        () => undefined,
        () => undefined,
      )
      .then(async () => {
        this.waiting--;
        if (!this.waiting) await this.onIdle();
      });
    return done;
  }
}
