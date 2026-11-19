import { Fragment } from 'react';
import { useDesktopApp } from '../connections/context';
import {
  appleKeyboard,
  bindingKeys,
  shortcutBindings,
  shortcuts,
  type Binding,
  type Shortcut,
} from './shortcut-keys';

/**
 * The keys that zoom the desktop app's window, where what is under them, such as the editor, does
 * not zoom its own text with them.
 */
const WINDOW_ZOOM: readonly { label: string; bindings: Binding[] }[] = [
  { label: 'Zoom in', bindings: [{ key: '=', mod: true }] },
  { label: 'Zoom out', bindings: [{ key: '-', mod: true }] },
  { label: 'Reset zoom', bindings: [{ key: '0', mod: true }] },
];

/** Every keyboard shortcut there is here, with its keys on this keyboard. */
export function KeyboardShortcuts() {
  const apple = appleKeyboard();
  const desktop = useDesktopApp();
  const rows = [
    ...(Object.keys(shortcuts) as Shortcut[]).flatMap((action) => {
      const bindings = shortcutBindings(action, { apple, desktop });
      return bindings.length ? [{ label: shortcuts[action].label, bindings }] : [];
    }),
    ...(desktop ? WINDOW_ZOOM : []),
  ];
  const named = (binding: Binding) => bindingKeys(binding, apple).join('+');
  return (
    <div className="page">
      <header className="page-heading">
        <h1>Keyboard shortcuts</h1>
      </header>
      <dl className="shortcut-list">
        {rows.map(({ label, bindings }) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd aria-label={bindings.map(named).join(' or ')}>
              {bindings.map((binding, index) => (
                <Fragment key={named(binding)}>
                  {index > 0 && <span className="shortcut-or">or</span>}
                  {bindingKeys(binding, apple).map((key) => (
                    <kbd key={key}>{key}</kbd>
                  ))}
                </Fragment>
              ))}
            </dd>
          </div>
        ))}
      </dl>
      <div className="shortcut-existing muted small">
        In the message box: Enter to send, Shift+Enter for a new line, Shift+Tab to cycle
        permissions.
      </div>
    </div>
  );
}
