import { useQueryClient } from '@tanstack/react-query';
import { FolderOpen, RotateCw } from 'lucide-react';
import type { Gateway } from '../api/gateway';
import { gatewayKey, useConnection, useDesktopState, useRuntime } from '../connections/context';
import { useAction, type Action } from '../components/actions';
import { Button } from '../components/ui/button';
import { SwitchField } from '../components/ui/switch';
import { ErrorNotice, Field } from '../components/common';

/** Saves the desktop app's settings with a change applied, keeping how the app answers. */
function useSaveSettings() {
  const runtime = useRuntime();
  const queries = useQueryClient();
  const { account } = useConnection();
  const state = useDesktopState();
  const action = useAction();
  const settings = state.data?.settings;
  return {
    busy: action.busy || !settings,
    error: state.error ?? action.error,
    settings,
    save(change: Partial<Gateway['DesktopSettings']>) {
      if (!settings) return;
      void action.run(async () => {
        const next = await runtime.gateway.send<Gateway['DesktopState']>(
          'PUT',
          '/api/desktop/settings',
          { ...settings, ...change } satisfies Gateway['DesktopSettings'],
        );
        queries.setQueryData(gatewayKey(account?.device_id, '/api/desktop'), next);
      });
    },
  };
}

const CLOSE_ACTIONS: Record<Gateway['CloseAction'], string> = {
  ask: 'Ask',
  tray: 'Keep running in tray',
  quit: 'Quit',
};

/** The desktop app's own settings: what closing the window does, and checking for updates. */
export function DesktopAppSettings() {
  const saving = useSaveSettings();
  return (
    <section className="panel">
      <h2>Desktop app</h2>
      <div className="form-stack">
        <div className="field-grid">
          <Field label="Closing the window">
            <select
              value={saving.settings?.close ?? 'ask'}
              disabled={saving.busy}
              onChange={(event) => {
                const close = closeAction(event.target.value);
                if (close) saving.save({ close });
              }}
            >
              {Object.entries(CLOSE_ACTIONS).map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <SwitchField
          label="Check for updates"
          checked={saving.settings?.check_updates ?? false}
          disabled={saving.busy}
          onCheckedChange={(checked) => saving.save({ check_updates: checked })}
        />
      </div>
      <p className="muted small">Asks GitHub once a day for a newer release.</p>
      <ErrorNotice error={saving.error} />
    </section>
  );
}

function closeAction(value: string): Gateway['CloseAction'] | undefined {
  return Object.keys(CLOSE_ACTIONS).find(
    (action): action is Gateway['CloseAction'] => action === value,
  );
}

/** System notifications in the desktop app, instead of push notifications. */
export function DesktopNotificationSettings() {
  const runtime = useRuntime();
  const saving = useSaveSettings();
  const test = useAction();
  const enabled = saving.settings?.notifications ?? false;
  return (
    <>
      <div className="notification-settings-row">
        <SwitchField
          label="System notifications"
          id="system-notifications"
          aria-describedby="notification-description"
          checked={enabled}
          disabled={saving.busy}
          onCheckedChange={(checked) => saving.save({ notifications: checked })}
        />
        <Button
          variant="secondary"
          size="sm"
          aria-label="Show a test notification"
          disabled={!enabled || test.busy}
          onClick={() =>
            void test.run(() => runtime.gateway.send('POST', '/api/desktop/notifications/test'))
          }
        >
          Test
        </Button>
      </div>
      <p id="notification-description" className="muted notification-description">
        When a turn ends or waits for approval, unless the window shows it.
      </p>
      <ErrorNotice error={saving.error ?? test.error} />
    </>
  );
}

const PROCESS_STATES: Record<Gateway['ProcessState'], { label: string; tone?: string }> = {
  starting: { label: 'Starting' },
  running: { label: 'Running', tone: 'success' },
  restarting: { label: 'Restarting' },
  stopped: { label: 'Stopped', tone: 'danger' },
};

/** The Meka the desktop app runs, as rows of About's facts, the log folder opened by `folders`. */
export function MekaProcessFacts({ folders }: { folders: Action }) {
  const state = useDesktopState(5000);
  const meka = state.data?.meka;
  if (!meka) return null;
  const shown = PROCESS_STATES[meka.state];
  return (
    <>
      <dt>Process</dt>
      <dd>
        <span className={shown.tone ? `badge ${shown.tone}` : 'badge'}>{shown.label}</span>
        {meka.pid !== null && meka.pid !== undefined && (
          <span className="muted small"> PID {meka.pid}</span>
        )}
        {meka.last_exit && <span className="muted small"> · Last ended with {meka.last_exit}</span>}
      </dd>
      <dt>Log folder</dt>
      <dd>
        {state.data?.logs}
        <OpenFolderButton folder="logs" action={folders} inline />
      </dd>
    </>
  );
}

/** That the desktop app is a portable copy, as a row of About's facts. */
export function PortableFact() {
  return useDesktopState().data?.portable ? (
    <>
      <dt>Mode</dt>
      <dd>Portable</dd>
    </>
  ) : null;
}

/** A newer release of Meka Studio, if the desktop app found one. */
export function UpdateNotice() {
  const update = useDesktopState().data?.update;
  return update ? (
    <span className="muted">
      {' '}
      ·{' '}
      <a href={update.url} target="_blank" rel="noreferrer noopener">
        Meka Studio {update.version} is available ↗
      </a>
    </span>
  ) : null;
}

/** Restarting the Meka the desktop app runs, as `action`, whose error is for the caller to show. */
export function RestartMekaButton({ action }: { action: Action }) {
  const runtime = useRuntime();
  const queries = useQueryClient();
  const { account } = useConnection();
  return (
    <Button
      variant="secondary"
      disabled={action.busy}
      onClick={() =>
        void action.run(async () => {
          await runtime.gateway.send('POST', '/api/desktop/meka/restart');
          await queries.invalidateQueries({
            queryKey: gatewayKey(account?.device_id, '/api/desktop'),
          });
        })
      }
    >
      <RotateCw size={16} className={action.busy ? 'spin' : undefined} />
      Restart Meka
    </Button>
  );
}

const FOLDER_NAMES: Record<Gateway['Folder'], string> = {
  config: 'config folder',
  data: 'data folder',
  logs: 'log folder',
};

/**
 * Opening one of the desktop app's folders in the file manager, as `action`, whose error is for the
 * caller to show: a button that names it, or with `inline`, an icon after the folder's path.
 */
export function OpenFolderButton({
  folder,
  action,
  inline = false,
}: {
  folder: Gateway['Folder'];
  action: Action;
  inline?: boolean;
}) {
  const runtime = useRuntime();
  const label = `Open ${FOLDER_NAMES[folder]}`;
  return (
    <Button
      variant={inline ? 'ghost' : 'secondary'}
      size={inline ? 'icon' : 'default'}
      className={inline ? 'open-folder' : undefined}
      aria-label={label}
      title={label}
      disabled={action.busy}
      onClick={() =>
        void action.run(() => runtime.gateway.send('POST', `/api/desktop/folders/${folder}/open`))
      }
    >
      <FolderOpen size={16} aria-hidden="true" />
      {!inline && label}
    </Button>
  );
}
