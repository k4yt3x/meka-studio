import { describe, expect, it } from 'vitest';
import {
  adjacentSession,
  bindingAria,
  bindingKeys,
  matchesShortcut,
  shortcutAllowed,
  shortcutBindings,
  shortcuts,
  type Shortcut,
} from './shortcut-keys';

const pc = { apple: false, desktop: false };
const mac = { apple: true, desktop: false };
/** The first keys of `action`, as its hint shows them. */
const first = (action: Shortcut) => shortcuts[action].keys[0]!;

const key = (changes: Partial<KeyboardEvent> = {}) => ({
  key: 'o',
  code: 'KeyO',
  ctrlKey: true,
  metaKey: false,
  altKey: false,
  shiftKey: true,
  ...changes,
});

describe('shortcut matching', () => {
  it('uses the platform modifier and accepts shifted letter case', () => {
    expect(matchesShortcut('newSession', key({ key: 'O' }), pc)).toBe(true);
    expect(matchesShortcut('newSession', key(), mac)).toBe(false);
    expect(matchesShortcut('newSession', key({ ctrlKey: false, metaKey: true }), mac)).toBe(true);
    expect(matchesShortcut('newSession', key({ altKey: true }), pc)).toBe(false);
    expect(matchesShortcut('newSession', key({ metaKey: true }), pc)).toBe(false);
  });

  it('does not treat word deletion, forward deletion, or plain Backspace as session deletion', () => {
    expect(matchesShortcut('deleteSession', key({ key: 'Backspace', code: 'Backspace' }), pc)).toBe(
      true,
    );
    expect(matchesShortcut('deleteSession', key({ key: 'Backspace', shiftKey: false }), pc)).toBe(
      false,
    );
    expect(matchesShortcut('deleteSession', key({ key: 'Backspace', ctrlKey: false }), pc)).toBe(
      false,
    );
    expect(matchesShortcut('deleteSession', key({ key: 'Delete' }), pc)).toBe(false);
  });

  it('recognizes macOS Option-produced characters without hijacking unmodified typing', () => {
    const optionR = key({
      key: '®',
      code: 'KeyR',
      ctrlKey: false,
      metaKey: true,
      altKey: true,
      shiftKey: false,
    });
    expect(matchesShortcut('renameSession', optionR, mac)).toBe(true);
    expect(matchesShortcut('renameSession', { ...optionR, metaKey: false }, mac)).toBe(false);
    expect(matchesShortcut('renameSession', { ...optionR, shiftKey: true }, mac)).toBe(false);
    expect(matchesShortcut('renameSession', { ...optionR, code: 'KeyP' }, mac)).toBe(false);
  });

  it('follows the typed letter on non-US layouts and supports a shifted slash', () => {
    expect(matchesShortcut('newSession', key({ code: 'KeyQ' }), pc)).toBe(true);
    expect(matchesShortcut('newSession', key({ key: 'p' }), pc)).toBe(false);
    expect(matchesShortcut('showShortcuts', key({ key: '/', code: 'Digit7' }), pc)).toBe(true);
    expect(matchesShortcut('showShortcuts', key({ key: '?', code: 'Slash' }), pc)).toBe(false);
    expect(
      matchesShortcut(
        'renameSession',
        key({ key: '®', code: 'KeyR', altKey: true, shiftKey: false }),
        pc,
      ),
    ).toBe(false);
  });

  it('requires only Alt for session navigation', () => {
    const up = key({
      key: 'ArrowUp',
      code: 'ArrowUp',
      altKey: true,
      ctrlKey: false,
      shiftKey: false,
    });
    expect(matchesShortcut('previousSession', up, pc)).toBe(true);
    expect(matchesShortcut('previousSession', up, mac)).toBe(true);
    expect(matchesShortcut('previousSession', { ...up, shiftKey: true }, pc)).toBe(false);
    expect(matchesShortcut('nextSession', { ...up, key: 'ArrowDown' }, pc)).toBe(true);
  });

  it('toggles the terminal with Control and backquote on every platform, by key or place', () => {
    const backquote = key({ key: '`', code: 'Backquote', shiftKey: false });
    expect(matchesShortcut('toggleTerminal', backquote, pc)).toBe(true);
    expect(matchesShortcut('toggleTerminal', backquote, mac)).toBe(true);
    expect(matchesShortcut('toggleTerminal', { ...backquote, key: 'Dead' }, pc)).toBe(true);
    // Where that key types a digit, as on a Hungarian keyboard, Ctrl with it is the browser's.
    expect(matchesShortcut('toggleTerminal', { ...backquote, key: '0' }, pc)).toBe(false);
    expect(
      matchesShortcut('toggleTerminal', { ...backquote, ctrlKey: false, metaKey: true }, mac),
    ).toBe(false);
    expect(matchesShortcut('toggleTerminal', { ...backquote, ctrlKey: false }, pc)).toBe(false);
    expect(bindingKeys(first('toggleTerminal'), true)).toEqual(['Ctrl', '`']);
    expect(bindingAria(first('toggleTerminal'), false)).toBe('Control+`');
    // With Shift, the key types a tilde: a new terminal, as in VS Code.
    const tilde = { ...backquote, key: '~', shiftKey: true };
    expect(matchesShortcut('newTerminal', tilde, pc)).toBe(true);
    expect(matchesShortcut('newTerminal', tilde, mac)).toBe(true);
    expect(matchesShortcut('toggleTerminal', tilde, pc)).toBe(false);
    expect(matchesShortcut('newTerminal', backquote, pc)).toBe(false);
  });

  it('copies and pastes in a terminal with Shift, leaving Ctrl+C and Ctrl+V to the shell', () => {
    const ctrlShiftC = key({ key: 'C', code: 'KeyC' });
    expect(matchesShortcut('copyFromTerminal', ctrlShiftC, pc)).toBe(true);
    expect(matchesShortcut('copyFromTerminal', { ...ctrlShiftC, shiftKey: false }, pc)).toBe(false);
    expect(matchesShortcut('pasteIntoTerminal', key({ key: 'V', code: 'KeyV' }), pc)).toBe(true);
    expect(matchesShortcut('pasteIntoTerminal', key({ key: 'v', shiftKey: false }), pc)).toBe(
      false,
    );
    expect(shortcutBindings('copyFromTerminal', mac)).toEqual([]);
  });

  it('finds in a terminal with Ctrl+Shift+F, or Cmd+F, leaving Ctrl+F to the shell', () => {
    const ctrlShiftF = key({ key: 'F', code: 'KeyF' });
    expect(matchesShortcut('findInTerminal', ctrlShiftF, pc)).toBe(true);
    expect(matchesShortcut('findInTerminal', { ...ctrlShiftF, shiftKey: false }, pc)).toBe(false);
    const cmdF = key({ key: 'f', code: 'KeyF', ctrlKey: false, metaKey: true, shiftKey: false });
    expect(matchesShortcut('findInTerminal', cmdF, mac)).toBe(true);
    expect(matchesShortcut('findInTerminal', ctrlShiftF, mac)).toBe(false);
  });

  it('closes a tab with Ctrl+Shift+W in the desktop app, as a terminal leaves it', () => {
    const ctrlShiftW = key({ key: 'W', code: 'KeyW' });
    expect(matchesShortcut('closeTab', ctrlShiftW, { ...pc, desktop: true })).toBe(true);
    expect(matchesShortcut('closeTab', ctrlShiftW, pc)).toBe(false);
    expect(matchesShortcut('closeTab', ctrlShiftW, { ...mac, desktop: true })).toBe(false);
  });

  it('keeps the desktop app’s keys out of browsers, which take them first', () => {
    const app = { ...pc, desktop: true };
    const ctrlN = key({ key: 'n', code: 'KeyN', shiftKey: false });
    expect(matchesShortcut('newSession', ctrlN, pc)).toBe(false);
    expect(matchesShortcut('newSession', ctrlN, app)).toBe(true);
    expect(matchesShortcut('newSession', key(), app)).toBe(true);
    const ctrlQ = key({ key: 'q', code: 'KeyQ', shiftKey: false });
    expect(matchesShortcut('quit', ctrlQ, pc)).toBe(false);
    expect(matchesShortcut('quit', ctrlQ, app)).toBe(true);
    expect(shortcutBindings('quit', pc)).toEqual([]);
    expect(shortcutBindings('newSession', pc)).toHaveLength(1);
  });

  it('takes each system’s own keys for going back and full screen', () => {
    const app = { ...pc, desktop: true };
    const macApp = { ...mac, desktop: true };
    const altLeft = key({
      key: 'ArrowLeft',
      code: 'ArrowLeft',
      ctrlKey: false,
      altKey: true,
      shiftKey: false,
    });
    expect(matchesShortcut('back', altLeft, app)).toBe(true);
    expect(matchesShortcut('back', altLeft, macApp)).toBe(false);
    const cmdBracket = key({
      key: '[',
      code: 'BracketLeft',
      ctrlKey: false,
      metaKey: true,
      shiftKey: false,
    });
    expect(matchesShortcut('back', cmdBracket, macApp)).toBe(true);
    expect(matchesShortcut('back', cmdBracket, app)).toBe(false);
    const f11 = key({ key: 'F11', code: 'F11', ctrlKey: false, shiftKey: false });
    expect(matchesShortcut('fullScreen', f11, app)).toBe(true);
    expect(matchesShortcut('fullScreen', f11, macApp)).toBe(false);
    const ctrlCmdF = key({ key: 'f', code: 'KeyF', metaKey: true, shiftKey: false });
    expect(matchesShortcut('fullScreen', ctrlCmdF, macApp)).toBe(true);
    expect(matchesShortcut('find', ctrlCmdF, macApp)).toBe(false);
    expect(matchesShortcut('find', { ...ctrlCmdF, ctrlKey: false }, macApp)).toBe(true);
  });

  it('steps through the tabs with Control and Tab, back with Shift', () => {
    const app = { ...mac, desktop: true };
    const ctrlTab = key({ key: 'Tab', code: 'Tab', shiftKey: false });
    expect(matchesShortcut('nextTab', ctrlTab, app)).toBe(true);
    expect(matchesShortcut('previousTab', ctrlTab, app)).toBe(false);
    expect(matchesShortcut('previousTab', { ...ctrlTab, shiftKey: true }, app)).toBe(true);
    expect(matchesShortcut('nextTab', { ...ctrlTab, key: 'PageDown' }, app)).toBe(true);
    expect(bindingKeys(first('previousTab'), true)).toEqual(['Ctrl', 'Shift', 'Tab']);
  });

  it('provides platform-specific display and accessible keys', () => {
    expect(bindingKeys(first('deleteSession'), true)).toEqual(['Cmd', 'Shift', '⌫']);
    expect(bindingAria(first('deleteSession'), true)).toBe('Meta+Shift+Backspace');
    expect(bindingAria(first('renameSession'), false)).toBe('Control+Alt+R');
    expect(bindingKeys(first('previousSession'), true)).toEqual(['Option', '↑']);
  });
});

describe('shortcut scope', () => {
  const context = {
    enabled: true,
    composing: false,
    repeated: false,
    overlay: false,
    editing: false,
    helpOpen: false,
  };
  it('blocks disabled, repeated and composing actions', () => {
    for (const patch of [{ enabled: false }, { repeated: true }, { composing: true }])
      expect(shortcutAllowed('deleteSession', { ...context, ...patch })).toBe(false);
  });
  it('gives dialogs and menus priority over session actions', () => {
    for (const action of [
      'deleteSession',
      'newSession',
      'stopTurn',
      'renameSession',
      'searchSessions',
    ] as const)
      expect(shortcutAllowed(action, { ...context, overlay: true })).toBe(false);
    expect(shortcutAllowed('showShortcuts', { ...context, overlay: true })).toBe(false);
    expect(shortcutAllowed('showShortcuts', { ...context, overlay: true, helpOpen: true })).toBe(
      true,
    );
    expect(shortcutAllowed('deleteSession', { ...context, overlay: true, helpOpen: true })).toBe(
      false,
    );
    for (const action of ['quit', 'reload', 'fullScreen'] as const)
      expect(shortcutAllowed(action, { ...context, overlay: true })).toBe(true);
  });
  it('preserves text navigation while permitting explicit composer actions', () => {
    expect(shortcutAllowed('previousSession', { ...context, editing: true })).toBe(false);
    expect(shortcutAllowed('nextSession', { ...context, editing: true })).toBe(false);
    // Alt with the arrows moves through no text on Windows and Linux, nor Cmd with brackets.
    expect(shortcutAllowed('back', { ...context, editing: true })).toBe(true);
    expect(shortcutAllowed('stopTurn', { ...context, editing: true })).toBe(true);
    expect(shortcutAllowed('newSession', { ...context, editing: true })).toBe(true);
  });
});

describe('session navigation order', () => {
  const displayed = ['pinned', 'parent', 'child', 'older'];
  it('uses the displayed order, including subagents', () => {
    expect(adjacentSession(displayed, 'parent', 1)).toBe('child');
    expect(adjacentSession(displayed, 'parent', -1)).toBe('pinned');
  });
  it('does not wrap, or jump to unrelated sessions when the current row is absent', () => {
    expect(adjacentSession(displayed, 'pinned', -1)).toBeUndefined();
    expect(adjacentSession(displayed, 'older', 1)).toBeUndefined();
    expect(adjacentSession(displayed, 'filtered-out', 1)).toBeUndefined();
    expect(adjacentSession(displayed, undefined, 1)).toBeUndefined();
    expect(adjacentSession([], 'parent', 1)).toBeUndefined();
  });
});
