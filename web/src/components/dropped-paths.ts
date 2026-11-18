/**
 * Where the files and folders dropped on the desktop app's window are on this computer, which a
 * webview keeps from the page. The app tells it on Linux, as the global `__MEKA_STUDIO_DROPPED__`
 * set before the drop reaches the page, and answers its asking on Windows, where WebView2 hands the
 * app the dropped files the page posts. macOS and browsers tell nothing.
 */

/** WebView2's channel to the app that hosts it. */
interface WebviewHost {
  postMessageWithAdditionalObjects?: (message: string, objects: readonly File[]) => void;
  addEventListener(type: 'message', listener: (event: MessageEvent) => void): void;
  removeEventListener(type: 'message', listener: (event: MessageEvent) => void): void;
}

type DesktopWindow = Window & {
  __MEKA_STUDIO_DROPPED__?: unknown;
  chrome?: { webview?: WebviewHost };
};

/** How long the app has to answer before the drop is taken as one without paths. */
const ANSWER_WAIT = 5000;

let asked = 0;

/**
 * The paths of what `transfer`, a drop on the window, brought, read during the drop event; none
 * where the app cannot tell, and an empty list where it was asked and could not answer.
 */
export function droppedPaths(transfer: DataTransfer): Promise<string[]> | undefined {
  const page = window as DesktopWindow;
  const told = page.__MEKA_STUDIO_DROPPED__;
  delete page.__MEKA_STUDIO_DROPPED__;
  if (isPaths(told) && told.length) return Promise.resolve(told);
  const host = page.chrome?.webview;
  const files = Array.from(transfer.files);
  if (!host?.postMessageWithAdditionalObjects || !files.length) return undefined;
  const id = ++asked;
  return new Promise((resolve) => {
    const timer = setTimeout(() => finish([]), ANSWER_WAIT);
    function answered(event: MessageEvent) {
      const answer: unknown = event.data;
      if (
        typeof answer === 'object' &&
        answer !== null &&
        'type' in answer &&
        answer.type === 'dropped-paths' &&
        'id' in answer &&
        answer.id === id &&
        'paths' in answer &&
        isPaths(answer.paths)
      )
        finish(answer.paths);
    }
    function finish(paths: string[]) {
      clearTimeout(timer);
      host?.removeEventListener('message', answered);
      resolve(paths);
    }
    host.addEventListener('message', answered);
    // A string: the app's first handler of a message, Tauri's, refuses any other, and WebView2
    // then hands it to no other.
    host.postMessageWithAdditionalObjects?.(JSON.stringify({ type: 'dropped-paths', id }), files);
  });
}

/**
 * Whether a drag offering addresses alone may be of files: WebKitGTK, the desktop app's webview on
 * Linux, offers what a file manager drags so, where other webviews offer the files.
 */
export function dropsAsAddresses(transfer: DataTransfer): boolean {
  return navigator.platform.startsWith('Linux') && transfer.types.includes('text/uri-list');
}

function isPaths(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((path) => typeof path === 'string');
}
