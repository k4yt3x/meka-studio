import { useEffect, useState } from 'react';
import { Copy, Minus, Square, X } from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** The window's minimize, maximize and close buttons, at its top right, over everything. */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    const current = getCurrentWindow();
    const check = () => {
      current.isMaximized().then(setMaximized, () => {});
    };
    check();
    addEventListener('resize', check);
    return () => removeEventListener('resize', check);
  }, []);
  return (
    <div className="window-controls">
      <button
        type="button"
        aria-label="Minimize"
        title="Minimize"
        onClick={() => void getCurrentWindow().minimize()}
      >
        <Minus size={16} aria-hidden="true" />
      </button>
      <button
        type="button"
        aria-label={maximized ? 'Restore' : 'Maximize'}
        title={maximized ? 'Restore' : 'Maximize'}
        onClick={() => void getCurrentWindow().toggleMaximize()}
      >
        {maximized ? (
          <Copy size={13} aria-hidden="true" className="window-restore" />
        ) : (
          <Square size={13} aria-hidden="true" />
        )}
      </button>
      <button
        type="button"
        className="window-close"
        aria-label="Close"
        title="Close"
        onClick={() => void getCurrentWindow().close()}
      >
        <X size={17} aria-hidden="true" />
      </button>
    </div>
  );
}
