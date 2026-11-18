import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import * as ContextMenu from '@radix-ui/react-context-menu';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import {
  AtSign,
  BookOpen,
  ChevronDown,
  ChevronUp,
  Code,
  Columns2,
  Copy,
  Download,
  ExternalLink,
  FilePlus,
  FileQuestion,
  FolderOpen,
  GitCompareArrows,
  LoaderCircle,
  MoreHorizontal,
  Plus,
  RotateCw,
  Save,
  SquareTerminal,
  WrapText,
  X,
} from 'lucide-react';
import { Button } from '../components/ui/button';
import { Dialog } from '../components/ui/dialog';
import type { MenuParts } from '../components/ui/menu';
import { useAction } from '../components/actions';
import { ErrorNotice, Field, Loading } from '../components/common';
import { Markdown } from '../components/markdown';
import { copyText } from '../components/clipboard';
import { useMediaQuery } from '../components/media-query';
import { useOverlayPadding } from '../components/visual-viewport';
import { useDarkTheme } from '../components/theme';
import { appleKeyboard, shortcutHint } from '../components/shortcut-keys';
import { focusMessage, mentionPath, useWorkspace } from '../components/workspace';
import { useRuntime, useSettings } from '../connections/context';
import { EDITOR_FONT } from '../connections/storage';
import { fileIconName, iconUrl } from '../files/icons';
import {
  basename,
  formatSize,
  isAbsolute,
  join,
  parent,
  rawUrl,
  relative,
  separator,
} from '../files/paths';
import { useFolderWatches } from '../files/watch';
import { useEditorStore } from '../editor/use-editor';
import {
  CompareSurface,
  EditorSurface,
  type CompareControls,
  type CompareProps,
} from '../editor/surface';
import {
  commitOf,
  commitTab,
  fileOf,
  terminalOf,
  terminalTab,
  type CommitFile,
  type EditorStore,
  type OpenFile,
} from '../editor/store';
import { ApiError } from '../api/client';
import { isProblem, type Gateway, type GatewayClient } from '../api/gateway';
import { TerminalView } from './terminal-view';
import { useCommittedText, useGitFile } from './git-status';
import { LineCounts } from './line-counts';

/**
 * The files and terminals open in a place, such as a session: a tab for each, and the one shown
 * below. Text opens in an editor, Monaco or, on a touch screen, CodeMirror; pictures, sound and
 * video show as they are; anything else can be downloaded. Changes are saved with Ctrl+S (⌘S), or
 * Save; a file changed on disk meanwhile takes what is there when it has no changes here, and
 * otherwise asks. A terminal's tab is its shell: closing it ends the shell. On a phone it covers
 * the screen, with the tabs in a menu.
 */
export function EditorPane({
  place,
  full,
  onHide,
}: {
  place: string;
  /** Over the whole screen, as on a phone. */
  full: boolean;
  /** Hides the pane, leaving its tabs open, as a phone's close button does. */
  onHide: () => void;
}) {
  const store = useEditorStore();
  const workspace = useWorkspace();
  const runtime = useRuntime();
  const queries = useQueryClient();
  const collisionPadding = useOverlayPadding();
  const tabs = store.tabs(place);
  const [closing, setClosing] = useState<readonly string[]>();
  const [closeError, setCloseError] = useState<unknown>();
  /** The path a new file is offered at, while one is being named. */
  const [newFile, setNewFile] = useState<string>();
  const { editorFontSize, terminalFontSize } = useSettings();
  // A size set with Ctrl and the wheel or keys lasts until the editor closes; Settings keeps the
  // sizes, the editor's and the terminals'.
  const [zoom, setZoom] = useState<number>();
  const fontSize = zoom ?? editorFontSize;
  const zoomBy = (step: -1 | 0 | 1) =>
    setZoom((size) =>
      step === 0
        ? undefined
        : Math.max(EDITOR_FONT.min, Math.min(EDITOR_FONT.max, (size ?? editorFontSize) + step)),
    );
  const [terminalZoom, setTerminalZoom] = useState<number>();
  // The tab shown is scrolled into sight, among tabs too many to fit.
  const strip = useRef<HTMLDivElement>(null);
  useEffect(() => {
    strip.current
      ?.querySelector('.code-tab[data-selected]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [tabs.active]);
  // A tab dragged with a mouse or pen shows, and moves among the others as it passes their middles.
  const drag = useRef<{ key: string; pointer: number; x: number; moved: boolean }>(undefined);
  // The click that ends a drag does not count as one on what it ends over.
  const dragged = useRef(false);
  function endDrag(pointer: number) {
    const current = drag.current;
    if (current?.pointer !== pointer) return;
    drag.current = undefined;
    if (!current.moved) return;
    dragged.current = true;
    setTimeout(() => (dragged.current = false));
  }
  // Changes to the open files' folders are looked at for the files themselves.
  useFolderWatches(store.folders(place), (folder) => void store.folderChanged(folder));
  const terminals = tabs.keys.flatMap((key) => {
    const id = terminalOf(key);
    return id === undefined ? [] : [id];
  });
  // Numbered as they started, where there are several.
  const started = [...terminals].sort(
    (a, b) =>
      Date.parse(store.terminal(a)?.created_at ?? '') -
      Date.parse(store.terminal(b)?.created_at ?? ''),
  );
  const terminalName = (id: string) =>
    started.length > 1 ? `Terminal ${started.indexOf(id) + 1}` : 'Terminal';
  /** What the tab `key` is called. */
  const name = (key: string) => {
    const id = terminalOf(key);
    const commit = commitOf(key);
    return id !== undefined
      ? terminalName(id)
      : commit
        ? `${basename(commit.path)} (${commit.commit.slice(0, 7)})`
        : basename(key);
  };
  /** Closes `keys`, asking first about files with changes not saved; terminals just end. */
  function close(keys: readonly string[]) {
    if (keys.some((key) => store.file(key)?.dirty)) setClosing(keys);
    else void closeTabs(keys);
  }
  /** Closes `keys`, the terminals among them ending their shells; those that cannot stay open. */
  async function closeTabs(keys: readonly string[]) {
    setCloseError(undefined);
    const failed = new Set<string>();
    const ids = keys.flatMap((key) => {
      const id = terminalOf(key);
      return id === undefined ? [] : [id];
    });
    await Promise.all(
      ids.map((id) =>
        closeTerminal(runtime.gateway, id).catch((error: unknown) => {
          failed.add(terminalTab(id));
          setCloseError(error);
        }),
      ),
    );
    store.close(
      place,
      keys.filter((key) => !failed.has(key)),
    );
    if (ids.length) void queries.invalidateQueries({ queryKey: ['gateway'] });
  }
  /** The terminal `id`'s shell ended: its tab goes, and the keyboard goes back to the message. */
  function ended(id: string, body: HTMLElement | null) {
    if (body?.contains(document.activeElement)) focusMessage();
    store.terminalEnded(id);
    void queries.invalidateQueries({ queryKey: ['gateway'] });
  }
  /** Asks for a new file's name, in the folder of the tab shown, else the working directory. */
  function startNewFile() {
    const shown = tabs.active;
    const id = shown === undefined ? undefined : terminalOf(shown);
    const commit = shown === undefined ? undefined : commitOf(shown);
    const folder =
      (shown === undefined
        ? undefined
        : id !== undefined
          ? store.terminal(id)?.cwd
          : commit
            ? parent(join(commit.repo, commit.path))
            : parent(shown)) ||
      workspace?.folder ||
      '~';
    setNewFile(/[\\/]$/.test(folder) ? folder : folder + separator(folder));
  }
  /** What the + menu after the tabs makes: a file, or a terminal. */
  const newItems = ({ Item }: MenuParts) => (
    <>
      <Item className="menu-item" onSelect={startNewFile}>
        <FilePlus size={14} />
        New file…
      </Item>
      {workspace?.openTerminal && (
        <Item className="menu-item" onSelect={() => workspace.openTerminal?.()}>
          <SquareTerminal size={14} />
          New terminal
          <span className="menu-item-hint">{shortcutHint('newTerminal')}</span>
        </Item>
      )}
    </>
  );
  const active = tabs.active ? store.file(tabs.active) : undefined;
  const activeCommit = tabs.active ? commitOf(tabs.active) : undefined;
  const items = (key: string, { Item, Separator }: MenuParts) => {
    const terminal = terminalOf(key);
    const commit = commitOf(key);
    const file = fileOf(key);
    const path =
      terminal !== undefined
        ? store.terminal(terminal)?.cwd
        : commit
          ? join(commit.repo, commit.path)
          : key;
    return (
      <>
        <Item className="menu-item" onSelect={() => close([key])}>
          <X size={14} />
          Close
        </Item>
        <Item
          className="menu-item"
          disabled={tabs.keys.length < 2}
          onSelect={() => close(tabs.keys.filter((each) => each !== key))}
        >
          Close others
        </Item>
        <Item className="menu-item" onSelect={() => close(tabs.keys)}>
          Close all
        </Item>
        <Separator className="menu-separator" />
        {terminal !== undefined && workspace?.openTerminal && (
          <Item className="menu-item" onSelect={() => workspace.openTerminal?.()}>
            <Plus size={14} />
            New terminal
            <span className="menu-item-hint">{shortcutHint('newTerminal')}</span>
          </Item>
        )}
        {file !== undefined && workspace && (
          <Item className="menu-item" onSelect={() => workspace.mention(mentionPath(file))}>
            <AtSign size={14} />
            Mention in message
          </Item>
        )}
        {terminal === undefined && path && workspace && (
          <Item className="menu-item" onSelect={() => workspace.showFile(path)}>
            <FolderOpen size={14} />
            Show in Files
          </Item>
        )}
        {file !== undefined && (
          <Item className="menu-item" asChild>
            <a href={rawUrl(file, true)} download>
              <Download size={14} />
              Download
            </a>
          </Item>
        )}
        {path && (
          <Item className="menu-item" onSelect={() => void copyText(path)}>
            <Copy size={14} />
            Copy path
          </Item>
        )}
      </>
    );
  };
  return (
    <section className="code-pane" data-layout={full ? 'full' : 'column'} aria-label="Editor">
      {full ? (
        <PhoneBar
          store={store}
          place={place}
          active={active}
          name={name}
          items={items}
          newItems={newItems}
          onHide={onHide}
        />
      ) : (
        <div
          ref={strip}
          className="code-tabs"
          role="tablist"
          aria-label="Open files and terminals"
          onWheel={(event) => {
            // A mouse's wheel scrolls the tabs sideways.
            if (!event.deltaX) event.currentTarget.scrollLeft += event.deltaY;
          }}
          onPointerDown={(event) => {
            dragged.current = false;
            drag.current = undefined;
            const target = event.target instanceof Element ? event.target : null;
            const tab = target?.closest<HTMLElement>('.code-tab')?.dataset.tab;
            // A finger scrolls the tabs instead.
            if (
              event.button !== 0 ||
              event.pointerType === 'touch' ||
              tab === undefined ||
              target?.closest('.code-tab-close')
            )
              return;
            drag.current = { key: tab, pointer: event.pointerId, x: event.clientX, moved: false };
          }}
          onPointerMove={(event) => {
            const current = drag.current;
            if (current?.pointer !== event.pointerId) return;
            // Let go outside the tabs, before it was dragged far enough to take them.
            if (!(event.buttons & 1)) {
              drag.current = undefined;
              return;
            }
            if (!current.moved) {
              if (Math.abs(event.clientX - current.x) < 5) return;
              current.moved = true;
              event.currentTarget.setPointerCapture(event.pointerId);
              store.show(place, current.key);
            }
            // Its place is after every other tab whose middle the pointer has passed.
            const index = [
              ...event.currentTarget.querySelectorAll<HTMLElement>('.code-tab'),
            ].filter((each) => {
              if (each.dataset.tab === current.key) return false;
              const box = each.getBoundingClientRect();
              return box.left + box.width / 2 < event.clientX;
            }).length;
            if (store.tabs(place).keys.indexOf(current.key) !== index)
              store.move(place, current.key, index);
          }}
          onPointerUp={(event) => endDrag(event.pointerId)}
          onPointerCancel={(event) => endDrag(event.pointerId)}
          onLostPointerCapture={(event) => endDrag(event.pointerId)}
          onClickCapture={(event) => {
            if (!dragged.current) return;
            dragged.current = false;
            event.stopPropagation();
          }}
        >
          {tabs.keys.map((key) => (
            <Tab
              key={key}
              store={store}
              tab={key}
              name={name(key)}
              selected={key === tabs.active}
              onShow={() => store.show(place, key)}
              onClose={() => close([key])}
              items={items}
            />
          ))}
          <DropdownMenu.Root modal={false}>
            <DropdownMenu.Trigger asChild>
              <button
                type="button"
                className="code-tab-new"
                aria-label="New file or terminal"
                title="New file or terminal"
              >
                <Plus size={15} aria-hidden="true" />
              </button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content
                className="menu-panel"
                align="start"
                sideOffset={4}
                collisionPadding={collisionPadding}
              >
                {newItems(DropdownMenu)}
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </div>
      )}
      <ErrorNotice error={closeError} />
      {/* Terminals stay attached while another tab shows, to hear their shells end. */}
      {terminals.map((id) => (
        <TerminalBody
          key={id}
          store={store}
          id={id}
          name={terminalName(id)}
          shown={tabs.active === terminalTab(id)}
          fontSize={terminalZoom ?? terminalFontSize}
          items={items}
          onZoom={setTerminalZoom}
          onEnded={ended}
        />
      ))}
      {active && (
        <EditorBody
          key={active.serial}
          store={store}
          file={active}
          full={full}
          items={items}
          fontSize={fontSize}
          onZoom={zoomBy}
        />
      )}
      {activeCommit && (
        <CommitBody
          key={tabs.active}
          store={store}
          commit={activeCommit}
          items={items}
          fontSize={fontSize}
          onZoom={zoomBy}
        />
      )}
      {newFile !== undefined && (
        <NewFileDialog
          start={newFile}
          onClose={() => setNewFile(undefined)}
          onMade={(path) => {
            setNewFile(undefined);
            if (workspace) workspace.openFile(path);
            else store.open(place, path);
          }}
        />
      )}
      <CloseDialog
        store={store}
        keys={closing}
        onDone={(close) => {
          if (close && closing) void closeTabs(closing);
          setClosing(undefined);
        }}
      />
    </section>
  );
}

/** Closes the terminal `id`; one that ended already counts as closed. */
async function closeTerminal(gateway: GatewayClient, id: string) {
  try {
    await gateway.send('DELETE', `/api/terminals/${encodeURIComponent(id)}`);
  } catch (error) {
    if (!isProblem(error, 'not-found')) throw error;
  }
}

/** A tab: its icon and name, a dot while it has changes, and its close button. */
function Tab({
  store,
  tab,
  name,
  selected,
  onShow,
  onClose,
  items,
}: {
  store: EditorStore;
  tab: string;
  name: string;
  selected: boolean;
  onShow: () => void;
  onClose: () => void;
  items: (key: string, parts: MenuParts) => ReactNode;
}) {
  const dark = useDarkTheme();
  const collisionPadding = useOverlayPadding();
  const terminal = terminalOf(tab);
  const commit = commitOf(tab);
  const file = store.file(tab);
  return (
    <ContextMenu.Root modal={false}>
      <ContextMenu.Trigger asChild>
        <div
          className="code-tab"
          data-tab={tab}
          data-selected={selected || undefined}
          data-dirty={file?.dirty || undefined}
          data-deleted={file?.deleted || undefined}
          title={
            terminal !== undefined
              ? store.terminal(terminal)?.cwd
              : commit
                ? `${commit.path} at ${commit.commit.slice(0, 7)}`
                : tab
          }
          onMouseDown={(event) => {
            // Pressed over tabs too many to fit, the middle button would start the browser's own
            // scrolling, and the click that closes the tab would never come.
            if (event.button === 1) event.preventDefault();
          }}
          onAuxClick={(event) => {
            // A middle click closes it, as in a browser's tabs.
            if (event.button === 1) {
              event.preventDefault();
              onClose();
            }
          }}
        >
          <button
            type="button"
            role="tab"
            aria-selected={selected}
            className="code-tab-name"
            onClick={onShow}
          >
            {terminal === undefined ? (
              <img
                src={iconUrl(fileIconName(commit ? basename(commit.path) : name), dark)}
                alt=""
                draggable={false}
              />
            ) : (
              <SquareTerminal size={15} aria-hidden="true" />
            )}
            <span>{name}</span>
          </button>
          <button
            type="button"
            className="code-tab-close"
            aria-label={
              file?.dirty ? `Close ${name}, which has changes not saved` : `Close ${name}`
            }
            title="Close"
            onClick={onClose}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      </ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className="menu-panel" collisionPadding={collisionPadding}>
          {items(tab, ContextMenu)}
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}

/** On a phone, the bar over the tab shown: closing the editor, the tabs open, and saving. */
function PhoneBar({
  store,
  place,
  active,
  name,
  items,
  newItems,
  onHide,
}: {
  store: EditorStore;
  place: string;
  active: OpenFile | undefined;
  name: (key: string) => string;
  items: (key: string, parts: MenuParts) => ReactNode;
  newItems: (parts: MenuParts) => ReactNode;
  onHide: () => void;
}) {
  const collisionPadding = useOverlayPadding();
  const tabs = store.tabs(place);
  return (
    <header className="code-phone-bar">
      <Button variant="ghost" size="icon" aria-label="Close the editor" onClick={onHide}>
        <X size={18} />
      </Button>
      <DropdownMenu.Root modal={false}>
        <DropdownMenu.Trigger asChild>
          <button type="button" className="code-phone-files" aria-label="Open files and terminals">
            <span>
              {active?.dirty ? '• ' : ''}
              {tabs.active && name(tabs.active)}
            </span>
            {tabs.keys.length > 1 && (
              <span className="muted small">{tabs.keys.length.toLocaleString()} open</span>
            )}
            <ChevronDown size={14} aria-hidden="true" />
          </button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content
            className="menu-panel"
            align="start"
            sideOffset={4}
            collisionPadding={collisionPadding}
          >
            {tabs.keys.map((key) => (
              <DropdownMenu.Item
                key={key}
                className="menu-item"
                onSelect={() => store.show(place, key)}
              >
                {store.file(key)?.dirty ? '• ' : ''}
                {name(key)}
              </DropdownMenu.Item>
            ))}
            <DropdownMenu.Separator className="menu-separator" />
            {newItems(DropdownMenu)}
            {tabs.active && (
              <>
                <DropdownMenu.Separator className="menu-separator" />
                {items(tabs.active, DropdownMenu)}
              </>
            )}
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
      {active?.kind === 'text' && (
        <Button
          size="sm"
          disabled={!active.dirty || active.saving}
          onClick={() => void store.save(active.path)}
        >
          <Save size={14} />
          Save
        </Button>
      )}
    </header>
  );
}

/** The "…" menu over what a tab shows. */
function MoreMenu({ label, children }: { label: string; children: ReactNode }) {
  const collisionPadding = useOverlayPadding();
  return (
    <DropdownMenu.Root modal={false}>
      <DropdownMenu.Trigger asChild>
        <Button variant="ghost" size="icon" aria-label={label} title="More">
          <MoreHorizontal size={16} />
        </Button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          className="menu-panel"
          align="end"
          sideOffset={4}
          collisionPadding={collisionPadding}
        >
          {children}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

/** A terminal's tab shown: the folder it started in, and the terminal. */
function TerminalBody({
  store,
  id,
  name,
  shown,
  fontSize,
  items,
  onZoom,
  onEnded,
}: {
  store: EditorStore;
  id: string;
  name: string;
  shown: boolean;
  fontSize: number;
  items: (key: string, parts: MenuParts) => ReactNode;
  onZoom: (size: number | undefined) => void;
  onEnded: (id: string, body: HTMLElement | null) => void;
}) {
  const body = useRef<HTMLDivElement>(null);
  const cwd = store.terminal(id)?.cwd ?? '';
  return (
    <div ref={body} className="code-terminal" hidden={!shown}>
      <div className="code-file-bar">
        <span className="code-file-path" title={cwd}>
          {`\u200e${cwd}\u200e`}
        </span>
        <MoreMenu label={`More for ${name}`}>{items(terminalTab(id), DropdownMenu)}</MoreMenu>
      </div>
      <TerminalView
        id={id}
        active={shown}
        asked={store.asked(terminalTab(id))}
        fontSize={fontSize}
        takeFocus={() => store.takeFocus(terminalTab(id))}
        onZoom={onZoom}
        onEnded={() => onEnded(id, body.current)}
      />
    </div>
  );
}

/** The file shown: its editor, or what it is, with what can be done with it. */
function EditorBody({
  store,
  file,
  full,
  items,
  fontSize,
  onZoom,
}: {
  store: EditorStore;
  file: OpenFile;
  /** Over the whole screen, with the phone's bar, which saves. */
  full: boolean;
  items: (key: string, parts: MenuParts) => ReactNode;
  fontSize: number;
  onZoom: (step: -1 | 0 | 1) => void;
}) {
  const coarse = useMediaQuery('(pointer: coarse)');
  // The editor loaded first stays, holding the files' changes, even if the pointer changes.
  const touch = store.documents ? store.documents.touch : coarse;
  const dark = useDarkTheme();
  const runtime = useRuntime();
  const { editorWordWrap: wrap, compareSideBySide } = useSettings();
  const [cursor, setCursor] = useState<{ line: number; column: number }>();
  const [stats, setStats] = useState<[number, number]>();
  const controlsRef = useRef<CompareControls | null>(null);
  const [highlighted, setHighlighted] = useState<string>();
  const [indentation, setIndentation] = useState<string>();
  const [comparing, setComparing] = useState(false);
  const markdown = /\.(md|markdown|mdx)$/i.test(file.name);
  const [previewing, setPreviewing] = useState(false);
  const readOnly = file.version === null && file.truncated;
  const text = file.kind === 'text' && file.state === 'ready';
  const conflict = file.conflict;
  const showCompare = comparing && conflict?.kind === 'changed' && !touch;
  // Shown beside the last commit, the file is in this repository.
  const changesIn = store.changesOf(file.path);
  const git = useGitFile(file.path, changesIn);
  // A file deleted since the last commit shows as all taken out.
  const gone =
    changesIn !== undefined &&
    file.state === 'failed' &&
    file.error instanceof ApiError &&
    file.error.status === 404;
  const showsComparison = gone || (text && !previewing && (showCompare || changesIn !== undefined));
  if (!showsComparison && stats) setStats(undefined);
  const toggleWrap = () => runtime.storage.editorWordWrap(!wrap);
  const save = () => void store.save(file.path);
  // A file put back after a reload is read as it shows.
  useEffect(() => store.wake(file.path), [store, file.path]);
  const bar = (
    <div className="code-file-bar">
      <span className="code-file-path" title={file.path}>
        {/* Marks hold the path's slashes in place, as it is cut short from its start. */}
        {`\u200e${file.path}\u200e`}
      </span>
      {text && markdown && (
        <Button size="sm" variant="ghost" onClick={() => setPreviewing(!previewing)}>
          {previewing ? <Code size={14} /> : <BookOpen size={14} />}
          {previewing ? 'Edit' : 'Preview'}
        </Button>
      )}
      {showsComparison && <CompareTools stats={stats} controlsRef={controlsRef} touch={touch} />}
      {(changesIn !== undefined || (text && git.change)) && (
        <Button
          size="icon"
          variant="ghost"
          aria-label="Changes since the last commit"
          aria-pressed={changesIn !== undefined}
          title="Changes since the last commit"
          onClick={() =>
            store.showChanges(file.path, changesIn === undefined ? git.repo : undefined)
          }
        >
          <GitCompareArrows size={15} />
        </Button>
      )}
      {/* A touch screen's editor always wraps. */}
      {text && !touch && (
        <Button
          size="icon"
          variant="ghost"
          className="wrap-toggle"
          aria-label="Word wrap"
          aria-pressed={wrap}
          title={`Word wrap (${appleKeyboard() ? '⌥Z' : 'Alt+Z'})`}
          onClick={toggleWrap}
        >
          <WrapText size={15} />
        </Button>
      )}
      {text && !full && (
        <Button
          size="icon"
          variant={file.dirty ? 'default' : 'ghost'}
          aria-label="Save"
          aria-busy={file.saving}
          disabled={!file.dirty || file.saving || readOnly}
          title={`Save (${appleKeyboard() ? '⌘S' : 'Ctrl+S'})`}
          onClick={save}
        >
          {file.saving ? <LoaderCircle size={15} className="spin" /> : <Save size={15} />}
        </Button>
      )}
      <MoreMenu label={`More for ${file.name}`}>{items(file.path, DropdownMenu)}</MoreMenu>
    </div>
  );
  if (file.state === 'loading')
    return (
      <>
        {bar}
        <Loading label={`Opening ${file.name}…`} fill />
      </>
    );
  const compare = {
    name: file.name,
    modified: { store, file },
    readOnly,
    dark,
    fontSize,
    wrap,
    sideBySide: compareSideBySide,
    onZoom,
    onSave: save,
    controlsRef,
    onStats: (added: number, removed: number) =>
      setStats((before) =>
        before?.[0] === added && before[1] === removed ? before : [added, removed],
      ),
    touch,
  };
  if (gone)
    return (
      <>
        {bar}
        <ChangesView {...compare} modified={{ text: '' }} file={file} git={git} />
      </>
    );
  if (file.state === 'failed')
    return (
      <>
        {bar}
        <div className="code-empty">
          <ErrorNotice error={file.error} />
          <Button size="sm" variant="secondary" onClick={() => void store.load(file.path)}>
            <RotateCw size={14} />
            Try again
          </Button>
        </div>
      </>
    );
  if (file.kind !== 'text')
    return (
      <>
        {bar}
        <Viewer file={file} />
      </>
    );
  return (
    <>
      {bar}
      {conflict?.kind === 'changed' && (
        <div className="code-conflict" role="status">
          <span>
            {showCompare ? 'On disk, left; yours, right.' : 'Changed on disk since it was opened.'}
          </span>
          {!touch && (
            <Button size="sm" variant="ghost" onClick={() => setComparing(!comparing)}>
              {comparing ? 'Stop comparing' : 'Compare'}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={() => void store.reload(file.path)}>
            Reload
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              setComparing(false);
              store.keep(file.path);
            }}
          >
            Keep mine
          </Button>
        </div>
      )}
      {conflict?.kind === 'deleted' && (
        <div className="code-conflict" role="status">
          <span>Deleted on disk. Saving makes it again.</span>
          <Button size="sm" variant="ghost" onClick={() => void store.reload(file.path)}>
            Close
          </Button>
          <Button size="sm" variant="ghost" onClick={() => store.keep(file.path)}>
            Keep
          </Button>
        </div>
      )}
      <ErrorNotice error={file.saveError} />
      {previewing ? (
        <div className="code-preview">
          <Markdown
            text={store.documents?.text(file.path, '\n')?.text ?? file.text}
            base={parent(file.path)}
          />
        </div>
      ) : showCompare ? (
        <CompareSurface {...compare} original={conflict.text} />
      ) : changesIn !== undefined ? (
        <ChangesView {...compare} file={file} git={git} />
      ) : (
        <EditorSurface
          touch={touch}
          store={store}
          file={file}
          asked={store.asked(file.path)}
          readOnly={readOnly}
          dark={dark}
          fontSize={fontSize}
          wrap={wrap}
          onZoom={onZoom}
          onWrap={toggleWrap}
          onSave={save}
          onLanguage={setHighlighted}
          onIndentation={setIndentation}
          onCursor={(line, column) =>
            setCursor((at) => (at?.line === line && at.column === column ? at : { line, column }))
          }
        />
      )}
      <div className="code-status">
        {readOnly && (
          <span title="Download it for the rest">
            The first {formatSize(new TextEncoder().encode(file.text).length)}; read only
          </span>
        )}
        <span className="code-status-gap" />
        {cursor && (
          <span>
            Ln {cursor.line}, Col {cursor.column}
          </span>
        )}
        {indentation && <span>{indentation}</span>}
        <span>{file.eol === '\r\n' ? 'CRLF' : 'LF'}</span>
        {highlighted && <span>{highlighted}</span>}
      </div>
    </>
  );
}

/**
 * The file beside what the last commit of its repository holds there, or where it was then, if it
 * was renamed since; a file the commit has none of, as one new since, beside nothing.
 */
function ChangesView({
  git,
  file,
  ...compare
}: Omit<CompareProps, 'original'> & {
  touch: boolean;
  git: ReturnType<typeof useGitFile>;
  file: OpenFile;
}) {
  const repo = git.repo ?? '';
  const committed = useCommittedText(
    repo,
    git.status.data?.commit ?? undefined,
    git.change?.from ?? relative(file.path, repo),
  );
  return (
    <CommittedCompare
      {...compare}
      error={git.status.error ?? committed.error}
      original={git.status.data ? committed.text : undefined}
      notText={committed.binary}
      tooLarge={committed.tooLarge || (file.truncated && file.version === null)}
    />
  );
}

/**
 * A comparison with what a commit holds, once it is read: what kept it from being read, or why it
 * cannot be compared, in its place.
 */
function CommittedCompare({
  error,
  original,
  notText,
  tooLarge,
  ...compare
}: Omit<CompareProps, 'original'> & {
  touch: boolean;
  error: unknown;
  /** None while it is read. */
  original: string | undefined;
  notText: boolean;
  tooLarge: boolean;
}) {
  if (error)
    return (
      <div className="code-empty">
        <ErrorNotice error={error} />
      </div>
    );
  if (notText || tooLarge)
    return (
      <div className="code-empty">
        <FileQuestion size={28} aria-hidden="true" />
        <p>{tooLarge ? 'Too large to compare.' : 'Not text, to compare.'}</p>
      </div>
    );
  if (original === undefined) return <Loading label="Reading the commit…" fill />;
  return <CompareSurface {...compare} original={original} />;
}

/** A comparison's lines added and removed, and the buttons that move through its changes. */
function CompareTools({
  stats,
  controlsRef,
  touch,
}: {
  stats: [number, number] | undefined;
  controlsRef: RefObject<CompareControls | null>;
  touch: boolean;
}) {
  const runtime = useRuntime();
  const { compareSideBySide } = useSettings();
  return (
    <>
      {stats && <LineCounts added={stats[0]} removed={stats[1]} />}
      <Button
        size="icon"
        variant="ghost"
        aria-label="Previous change"
        title="Previous change"
        onClick={() => controlsRef.current?.previous()}
      >
        <ChevronUp size={15} />
      </Button>
      <Button
        size="icon"
        variant="ghost"
        aria-label="Next change"
        title="Next change"
        onClick={() => controlsRef.current?.next()}
      >
        <ChevronDown size={15} />
      </Button>
      {/* A touch screen's comparison is always one column. */}
      {!touch && (
        <Button
          size="icon"
          variant="ghost"
          aria-label="Side by side"
          aria-pressed={compareSideBySide}
          title="Side by side"
          onClick={() => runtime.storage.compareSideBySide(!compareSideBySide)}
        >
          <Columns2 size={15} />
        </Button>
      )}
    </>
  );
}

/** A file as a commit holds it, beside what the commit before it held there, to read. */
function CommitBody({
  store,
  commit,
  items,
  fontSize,
  onZoom,
}: {
  store: EditorStore;
  commit: CommitFile;
  items: (key: string, parts: MenuParts) => ReactNode;
  fontSize: number;
  onZoom: (step: -1 | 0 | 1) => void;
}) {
  const coarse = useMediaQuery('(pointer: coarse)');
  const touch = store.documents ? store.documents.touch : coarse;
  const dark = useDarkTheme();
  const { editorWordWrap: wrap, compareSideBySide } = useSettings();
  const [stats, setStats] = useState<[number, number]>();
  const controlsRef = useRef<CompareControls | null>(null);
  const before = useCommittedText(
    commit.repo,
    commit.parent ?? undefined,
    commit.from ?? commit.path,
  );
  const after = useCommittedText(commit.repo, commit.commit, commit.path);
  const name = basename(commit.path);
  return (
    <>
      <div className="code-file-bar">
        <span className="code-file-path" title={`${commit.path} at ${commit.commit}`}>
          {`\u200e${commit.path}\u200e`}
        </span>
        <CompareTools stats={stats} controlsRef={controlsRef} touch={touch} />
        <MoreMenu label={`More for ${name}`}>{items(commitTab(commit), DropdownMenu)}</MoreMenu>
      </div>
      <CommittedCompare
        name={name}
        original={after.text === undefined ? undefined : before.text}
        modified={{ text: after.text ?? '' }}
        readOnly
        dark={dark}
        fontSize={fontSize}
        wrap={wrap}
        sideBySide={compareSideBySide}
        onZoom={onZoom}
        controlsRef={controlsRef}
        onStats={(added, removed) => setStats([added, removed])}
        touch={touch}
        error={before.error ?? after.error}
        notText={before.binary || after.binary}
        tooLarge={before.tooLarge || after.tooLarge}
      />
    </>
  );
}

/** A file that is not text to edit: shown as it is, where a browser can, or offered to download. */
function Viewer({ file }: { file: OpenFile }) {
  const url = rawUrl(file.path);
  return (
    <div className={`code-viewer code-viewer-${file.kind}`}>
      {file.kind === 'image' && <img src={url} alt={file.name} />}
      {file.kind === 'video' && <video src={url} controls playsInline />}
      {file.kind === 'audio' && <audio src={url} controls />}
      {file.kind === 'pdf' && (
        <div className="code-empty">
          <FileQuestion size={28} aria-hidden="true" />
          <p>PDFs open in a tab of their own.</p>
          <a
            className="button button-secondary button-small"
            href={url}
            target="_blank"
            rel="noreferrer noopener"
          >
            <ExternalLink size={14} />
            Open the PDF
          </a>
        </div>
      )}
      {file.kind === 'binary' && (
        <div className="code-empty">
          <FileQuestion size={28} aria-hidden="true" />
          <p>This file is not text.</p>
          <a
            className="button button-secondary button-small"
            href={rawUrl(file.path, true)}
            download
          >
            <Download size={14} />
            Download
          </a>
        </div>
      )}
    </div>
  );
}

/**
 * Asks where to make a new file, offering `start`, the folder of the tab shown; a name alone goes
 * there. It is made empty, never over a file already there, and then opens.
 */
function NewFileDialog({
  start,
  onClose,
  onMade,
}: {
  start: string;
  onClose: () => void;
  onMade: (path: string) => void;
}) {
  const runtime = useRuntime();
  const action = useAction();
  const [typed, setTyped] = useState(start);
  const text = typed.trim();
  const path = isAbsolute(text) || text.startsWith('~') ? text : join(start, text);
  const named = text !== '' && !/[\\/]$/.test(text);
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title="New file"
      description="It is made empty, and opens."
    >
      <form
        className="form-stack"
        onSubmit={(event) => {
          event.preventDefault();
          if (!named) return;
          void action
            .run(async () => {
              // Its folder as Files shows it, from `~`, `.` and `..`, for the tab to be the one
              // Files opens.
              const folder = await runtime.gateway.get<Gateway['Listing']>('/api/files/list', {
                path: parent(path),
              });
              const made = join(folder.path, basename(path));
              await runtime.gateway.send('PUT', '/api/files/text', {
                path: made,
                text: '',
                version: null,
              });
              return made;
            })
            .then((made) => {
              if (typeof made === 'string') onMade(made);
            });
        }}
      >
        <Field label="Path">
          <input
            autoFocus
            value={typed}
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            onFocus={(event) => {
              const end = event.currentTarget.value.length;
              event.currentTarget.setSelectionRange(end, end);
            }}
            onChange={(event) => setTyped(event.target.value)}
          />
        </Field>
        <ErrorNotice error={action.error} />
        <div className="actions">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={!named || action.busy}>
            Create
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/**
 * Asks what becomes of changes not saved in the files being closed. The other tabs closing with
 * them, terminals among them, close only if they do.
 */
function CloseDialog({
  store,
  keys,
  onDone,
}: {
  store: EditorStore;
  keys: readonly string[] | undefined;
  /** Called once it is settled, with whether to close them. */
  onDone: (close: boolean) => void;
}) {
  const [saving, setSaving] = useState(false);
  const dirty = (keys ?? []).filter((key) => store.file(key)?.dirty);
  const one = dirty.length === 1 ? basename(dirty[0]!) : undefined;
  return (
    <Dialog
      open={Boolean(keys)}
      onOpenChange={(open) => !open && onDone(false)}
      title="Changes not saved"
      description={
        one
          ? `Save the changes to ${one} before closing it?`
          : `Save the changes to ${dirty.length.toLocaleString()} files before closing them?`
      }
    >
      <div className="actions">
        <Button variant="secondary" onClick={() => onDone(false)}>
          Cancel
        </Button>
        <Button variant="secondary" tone="danger" onClick={() => onDone(true)}>
          Don't save
        </Button>
        <Button
          disabled={saving}
          onClick={() => {
            setSaving(true);
            void Promise.all(dirty.map((path) => store.save(path))).then((saved) => {
              setSaving(false);
              // Any that could not be saved stays open, saying why.
              if (saved.every(Boolean)) onDone(true);
              else onDone(false);
            });
          }}
        >
          Save
        </Button>
      </div>
    </Dialog>
  );
}
