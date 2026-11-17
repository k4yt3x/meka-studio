import { RAIL_WIDTH } from '../features/workspace-layout';

/**
 * How the desktop app's window draws its title bar, which the app sets before the page's scripts
 * run: `app` where the page draws the window's buttons (Windows), `overlay` where the system's
 * buttons sit over the page (macOS). Unset in a browser and where the system draws the bar. The
 * page's root element carries it as `data-title-bar`, for the styles.
 */
export const TITLE_BAR = (window as Window & { __MEKA_STUDIO_TITLE_BAR__?: string })
  .__MEKA_STUDIO_TITLE_BAR__;
if (TITLE_BAR) document.documentElement.dataset.titleBar = TITLE_BAR;
// Each of the window's buttons is as wide as the panels' rail, so the close button stands over it.
if (TITLE_BAR === 'app')
  document.documentElement.style.setProperty('--window-controls-width', `${3 * RAIL_WIDTH}px`);
