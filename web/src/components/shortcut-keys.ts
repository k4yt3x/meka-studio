export const shortcuts = {
  newSession: {
    label: 'New session',
    keys: [
      { key: 'o', mod: true, shift: true },
      { key: 'n', mod: true, desktop: true },
    ],
  },
  deleteSession: { label: 'Delete session', keys: [{ key: 'Backspace', mod: true, shift: true }] },
  renameSession: { label: 'Rename session', keys: [{ key: 'r', mod: true, alt: true }] },
  previousSession: { label: 'Previous session', keys: [{ key: 'ArrowUp', alt: true }] },
  nextSession: { label: 'Next session', keys: [{ key: 'ArrowDown', alt: true }] },
  stopTurn: { label: 'Stop turn', keys: [{ key: 'x', mod: true, shift: true }] },
  searchSessions: { label: 'Search sessions', keys: [{ key: 'k', mod: true }] },
  find: { label: 'Find in the conversation', keys: [{ key: 'f', mod: true, desktop: true }] },
  // Control on macOS too, as in editors; the key's place counts where `` ` `` is a dead key.
  toggleTerminal: {
    label: 'Toggle terminal',
    keys: [{ key: '`', code: 'Backquote', ctrl: true }],
  },
  newTerminal: {
    label: 'New terminal',
    keys: [{ key: '`', code: 'Backquote', ctrl: true, shift: true }],
  },
  closeTab: { label: 'Close tab', keys: [{ key: 'w', mod: true, desktop: true }] },
  // Control on macOS too, as Safari and VS Code have it.
  nextTab: {
    label: 'Next tab',
    keys: [
      { key: 'Tab', ctrl: true, desktop: true },
      { key: 'PageDown', ctrl: true, desktop: true },
    ],
  },
  previousTab: {
    label: 'Previous tab',
    keys: [
      { key: 'Tab', ctrl: true, shift: true, desktop: true },
      { key: 'PageUp', ctrl: true, desktop: true },
    ],
  },
  // Control on macOS too, as Cmd+Shift with a digit takes screenshots there. Browsers take most
  // letters with Ctrl+Shift, such as Firefox's network tools on E.
  toggleFiles: {
    label: 'Toggle Files',
    keys: [{ key: '1', code: 'Digit1', ctrl: true, shift: true }],
  },
  toggleGit: { label: 'Toggle Git', keys: [{ key: '2', code: 'Digit2', ctrl: true, shift: true }] },
  toggleContext: {
    label: 'Toggle Context',
    keys: [{ key: '3', code: 'Digit3', ctrl: true, shift: true }],
  },
  toggleTasks: {
    label: 'Toggle Tasks',
    keys: [{ key: '4', code: 'Digit4', ctrl: true, shift: true }],
  },
  toggleSchedules: {
    label: 'Toggle Schedules',
    keys: [{ key: '5', code: 'Digit5', ctrl: true, shift: true }],
  },
  back: {
    label: 'Back',
    keys: [
      { key: 'ArrowLeft', alt: true, system: 'other', desktop: true },
      { key: '[', code: 'BracketLeft', mod: true, system: 'apple', desktop: true },
    ],
  },
  forward: {
    label: 'Forward',
    keys: [
      { key: 'ArrowRight', alt: true, system: 'other', desktop: true },
      { key: ']', code: 'BracketRight', mod: true, system: 'apple', desktop: true },
    ],
  },
  reload: {
    label: 'Reload',
    keys: [
      { key: 'r', mod: true, desktop: true },
      { key: 'F5', desktop: true },
    ],
  },
  fullScreen: {
    label: 'Full screen',
    keys: [
      { key: 'F11', system: 'other', desktop: true },
      { key: 'f', ctrl: true, mod: true, system: 'apple', desktop: true },
    ],
  },
  openSettings: { label: 'Settings', keys: [{ key: ',', mod: true, desktop: true }] },
  showShortcuts: { label: 'Keyboard shortcuts', keys: [{ key: '/', mod: true }] },
  quit: { label: 'Quit', keys: [{ key: 'q', mod: true, desktop: true }] },
} satisfies Record<string, { label: string; keys: readonly Binding[] }>;

export type Shortcut = keyof typeof shortcuts;
/**
 * A shortcut's keys: `mod` is Cmd on macOS and Ctrl elsewhere, `ctrl` Control everywhere, and
 * `code` the key's place on the keyboard, matched as well as the character it types. A binding
 * with `system` is that system's alone, and one with `desktop` the desktop app's, where no browser
 * takes the keys first.
 */
export type Binding = {
  key: string;
  code?: string;
  mod?: boolean;
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
  system?: 'apple' | 'other';
  desktop?: boolean;
};
/** Where keys are pressed: on an Apple keyboard or not, and in the desktop app or a browser. */
export interface KeyContext {
  apple: boolean;
  desktop: boolean;
}
type KeyEvent = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>;

export function appleKeyboard() {
  return /Mac|iPhone|iPad|iPod/.test(navigator.platform);
}

/** The bindings of `action` that hold in `context`; none where it has no keys there. */
export function shortcutBindings(action: Shortcut, context: KeyContext): Binding[] {
  const keys: readonly Binding[] = shortcuts[action].keys;
  return keys.filter(
    (binding) =>
      (!binding.desktop || context.desktop) &&
      (!binding.system || (binding.system === 'apple') === context.apple),
  );
}

/** The keys of `action` a browser leaves it on this keyboard, as a hint beside what it does. */
export function shortcutHint(action: Shortcut) {
  const apple = appleKeyboard();
  const [binding] = shortcutBindings(action, { apple, desktop: false });
  return binding ? bindingKeys(binding, apple).join('+') : '';
}

/** The keys of `action` a browser leaves it on this keyboard, as `aria-keyshortcuts` names them. */
export function shortcutAttribute(action: Shortcut) {
  const apple = appleKeyboard();
  return shortcutBindings(action, { apple, desktop: false })
    .map((binding) => bindingAria(binding, apple))
    .join(' ');
}

export function matchesShortcut(action: Shortcut, event: KeyEvent, context: KeyContext) {
  return shortcutBindings(action, context).some((binding) =>
    matchesBinding(binding, event, context.apple),
  );
}

function matchesBinding(binding: Binding, event: KeyEvent, apple: boolean) {
  // Option can produce a different character on macOS (Option+R produces ®).
  const keyMatches =
    event.key.toLowerCase() === binding.key.toLowerCase() ||
    // By place only where the key types no letter or digit: a dead key, or a symbol.
    (event.code === binding.code && !/^[\p{L}\p{N}]$/u.test(event.key)) ||
    (apple && binding.alt && event.code === `Key${binding.key.toUpperCase()}`);
  return Boolean(
    keyMatches &&
    event.ctrlKey === Boolean((binding.mod && !apple) || binding.ctrl) &&
    event.metaKey === Boolean(binding.mod && apple) &&
    event.altKey === Boolean(binding.alt) &&
    (event.shiftKey === Boolean(binding.shift) || (binding.key === '/' && event.key === '/')),
  );
}

/** The keys `binding` names, as a keyboard shows them. */
export function bindingKeys(binding: Binding, apple: boolean): string[] {
  const names: Record<string, string> = {
    Backspace: apple ? '⌫' : 'Backspace',
    ArrowUp: '↑',
    ArrowDown: '↓',
    ArrowLeft: '←',
    ArrowRight: '→',
    PageUp: 'PgUp',
    PageDown: 'PgDn',
    Tab: 'Tab',
  };
  return [
    ...(binding.ctrl ? ['Ctrl'] : []),
    ...(binding.mod ? [apple ? 'Cmd' : 'Ctrl'] : []),
    ...(binding.alt ? [apple ? 'Option' : 'Alt'] : []),
    ...(binding.shift ? ['Shift'] : []),
    names[binding.key] ?? binding.key.toUpperCase(),
  ];
}

/** The keys `binding` names, as `aria-keyshortcuts` does. */
export function bindingAria(binding: Binding, apple: boolean) {
  return [
    ...(binding.ctrl ? ['Control'] : []),
    ...(binding.mod ? [apple ? 'Meta' : 'Control'] : []),
    ...(binding.alt ? ['Alt'] : []),
    ...(binding.shift ? ['Shift'] : []),
    binding.key.length === 1 ? binding.key.toUpperCase() : binding.key,
  ].join('+');
}

/**
 * Whether `action` leaves its keys to a field being typed in, where they move through the text or
 * through a control's choices, as Option with the arrows does on macOS.
 */
export function yieldsToTyping(action: Shortcut) {
  return action === 'previousSession' || action === 'nextSession';
}

/** Whether `action` is the window's own, which works whatever shows in it, a dialog too. */
function ofTheWindow(action: Shortcut) {
  return action === 'quit' || action === 'reload' || action === 'fullScreen';
}

export function shortcutAllowed(
  action: Shortcut,
  context: {
    enabled: boolean;
    composing: boolean;
    repeated: boolean;
    overlay: boolean;
    editing: boolean;
    helpOpen: boolean;
  },
) {
  if (!context.enabled || context.composing || context.repeated) return false;
  if (context.overlay && !ofTheWindow(action) && !(action === 'showShortcuts' && context.helpOpen))
    return false;
  return !(yieldsToTyping(action) && context.editing);
}

/** Navigation follows the displayed order, never wraps or guesses at unloaded rows. */
export function adjacentSession(ids: string[], current: string | undefined, direction: -1 | 1) {
  if (!current) return undefined;
  const index = ids.indexOf(current);
  return index < 0 ? undefined : ids[index + direction];
}
