import { useEffect, useEffectEvent } from 'react';
import type { Gateway } from '../api/gateway';
import { useDesktopApp, useRuntime } from '../connections/context';
import { appleKeyboard } from '../components/shortcut-keys';
import { zoomStep } from '../components/text-zoom';
import { useShortcut } from '../components/use-shortcut';

/** The mouse's back and forward buttons, as `MouseEvent.button` numbers them. */
const MOUSE_BACK = 3;
const MOUSE_FORWARD = 4;

/** The step of the window's zoom that `zoomStep`'s answer asks for. */
function zoomOf(step: -1 | 0 | 1): Gateway['ZoomStep'] {
  return step === 1 ? 'in' : step === -1 ? 'out' : 'reset';
}

/**
 * The desktop app's window keys, which a browser has of its own: quitting, reloading, going back
 * and forward through what was shown, as the mouse's side buttons do too, full screen, and
 * zooming, where what is under the keys does not zoom its own text with them.
 */
export function useDesktopWindowKeys() {
  const runtime = useRuntime();
  const desktopApp = useDesktopApp();
  /**
   * Asks the app for `path`. That fails only with the server gone, which the page shows by
   * itself, so a failure is only logged.
   */
  const ask = (path: string, body?: unknown) => {
    runtime.gateway.send('POST', path, body).catch((error: unknown) => {
      console.error(`Could not ask the app for ${path}:`, error);
    });
  };
  // The window's history starts at the app's start page, which the Navigation API leaves out as
  // another site's: there is nothing to go back to before Meka Studio's first page. Where the
  // webview has no Navigation API, the window does not go back.
  const back = () => {
    if (window.navigation?.canGoBack) history.back();
  };
  const forward = () => {
    if (window.navigation?.canGoForward) history.forward();
  };
  useShortcut('quit', () => ask('/api/desktop/quit'));
  useShortcut('reload', () => location.reload());
  useShortcut('back', back);
  useShortcut('forward', forward);
  useShortcut('fullScreen', () => ask('/api/desktop/window/full-screen'));
  const zoom = useEffectEvent((event: KeyboardEvent) => {
    if (event.defaultPrevented) return;
    const step = zoomStep(event, appleKeyboard());
    if (step === undefined) return;
    event.preventDefault();
    ask('/api/desktop/window/zoom', { step: zoomOf(step) });
  });
  const sideButton = useEffectEvent((event: MouseEvent) => {
    if (event.button !== MOUSE_BACK && event.button !== MOUSE_FORWARD) return;
    // Pressed, and let go, so the webview goes nowhere by itself.
    event.preventDefault();
    if (event.type !== 'mouseup') return;
    if (event.button === MOUSE_BACK) back();
    else forward();
  });
  useEffect(() => {
    if (!desktopApp) return;
    document.addEventListener('keydown', zoom);
    document.addEventListener('mousedown', sideButton);
    document.addEventListener('mouseup', sideButton);
    return () => {
      document.removeEventListener('keydown', zoom);
      document.removeEventListener('mousedown', sideButton);
      document.removeEventListener('mouseup', sideButton);
    };
  }, [desktopApp]);
}
