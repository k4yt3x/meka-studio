import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { Check, RotateCw, Search, ZoomIn, ZoomOut } from 'lucide-react';
import { Button } from '../components/ui/button';
import { ErrorNotice, Field, Loading } from '../components/common';
import { FindBar } from '../components/find-bar';
import { appleKeyboard } from '../components/shortcut-keys';
import { followZoomWheel, zoomStep } from '../components/text-zoom';
import { useOverlayPadding } from '../components/visual-viewport';
import type { EditorStore, OpenFile } from '../editor/store';
import { rawUrl } from '../files/paths';
import { pdfPlace, type Fit } from '../files/pdf-place';
import type { Matches, PdfState, PdfViewer } from '../files/pdf';

const FIT_NAMES: readonly (readonly [Fit, string])[] = [
  ['auto', 'Automatic'],
  ['page-width', 'Fit width'],
  ['page-fit', 'Fit page'],
  ['page-actual', 'Actual size'],
];

/**
 * A PDF, drawn by PDF.js, its page and zoom in the file's bar: Ctrl with the wheel, `=`, `-` or
 * `0`, or a pinch, zooms it, and Ctrl+F (⌘F) finds text in it. It is read again as it changes on
 * disk, staying where it was, and opens where it was left.
 */
export function PdfView({
  store,
  file,
  touch,
  bar,
}: {
  store: EditorStore;
  file: OpenFile;
  /** On a touch screen, where a pinch zooms, without the zoom buttons. */
  touch: boolean;
  /** The file's bar, with these tools in it. */
  bar: (tools: ReactNode) => ReactNode;
}) {
  const host = useRef<HTMLDivElement>(null);
  const container = useRef<HTMLDivElement>(null);
  const findField = useRef<HTMLInputElement>(null);
  const viewer = useRef<PdfViewer>(undefined);
  const [state, setState] = useState<PdfState>();
  const [error, setError] = useState<unknown>();
  const [asking, setAsking] = useState<{ wrong: boolean; answer: (password: string) => void }>();
  const [password, setPassword] = useState('');
  /** What is looked for, while finding. */
  const [query, setQuery] = useState<string>();
  const [matches, setMatches] = useState<Matches>();
  /** Goes up each time finding is asked for, to select what was typed again. */
  const [findAsked, setFindAsked] = useState(0);
  /** A page number being typed. */
  const [typed, setTyped] = useState<string>();
  /** Goes up each time it is tried again. */
  const [tries, setTries] = useState(0);
  const collisionPadding = useOverlayPadding();
  const path = file.path;
  const finding = query !== undefined;
  const asked = store.asked(path);
  /** Whether a password or an error shows in the PDF's place. */
  const covered = error !== undefined || asking !== undefined;

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let stopped = false;
    let made: PdfViewer | undefined;
    const keep = () => {
      const place = made?.place();
      if (place) store.keepView(path, place);
    };
    import('../files/pdf')
      .then(({ openPdf }) =>
        openPdf({
          container: element,
          url: rawUrl(path),
          place: pdfPlace(store.keptView(path)),
          onState: setState,
          onMatches: setMatches,
          onPassword: (wrong) =>
            new Promise((answer) =>
              setAsking({
                wrong,
                answer: (given) => {
                  setAsking(undefined);
                  setPassword('');
                  answer(given);
                },
              }),
            ),
          onError: setError,
        }),
      )
      .then((opened) => {
        if (stopped) opened.destroy();
        else {
          made = opened;
          viewer.current = opened;
        }
      })
      .catch(setError);
    // A reload leaves the page without this unmounting.
    window.addEventListener('pagehide', keep);
    return () => {
      stopped = true;
      window.removeEventListener('pagehide', keep);
      keep();
      made?.destroy();
      viewer.current = undefined;
    };
  }, [store, path, tries]);

  // Changed on disk, it is read again; one that could not be read, perhaps half written, is tried
  // again.
  const [seen, setSeen] = useState(file.revision);
  if (seen !== file.revision) {
    setSeen(file.revision);
    setError(undefined);
  }
  const revision = useRef(file.revision);
  useEffect(() => {
    if (file.revision === revision.current) return;
    revision.current = file.revision;
    viewer.current?.reload();
  }, [file.revision]);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const key = (event: KeyboardEvent) => {
      const apple = appleKeyboard();
      const command = apple ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
      if (command && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'f') {
        event.preventDefault();
        setQuery((text) => text ?? '');
        setFindAsked((count) => count + 1);
        return;
      }
      const step = zoomStep(event, apple);
      if (step === undefined) return;
      event.preventDefault();
      event.stopPropagation();
      if (step === 0) viewer.current?.fit('auto');
      else viewer.current?.zoom(step);
    };
    element.addEventListener('keydown', key, { capture: true });
    const stopWheel = followZoomWheel(element, (step, at) => viewer.current?.zoom(step, at));
    return () => {
      element.removeEventListener('keydown', key, { capture: true });
      stopWheel();
    };
  }, []);

  useEffect(() => {
    if (finding) findField.current?.select();
  }, [finding, findAsked]);

  // Asked for, as from Files, it takes the keyboard, as a text file's editor does; not one shown
  // as a session opens.
  useEffect(() => {
    if (store.takeFocus(path)) container.current?.focus({ preventScroll: true });
  }, [store, path, asked]);

  function find(text: string, again?: 'next' | 'previous') {
    setQuery(text);
    if (text) viewer.current?.find(text, again);
    else {
      viewer.current?.stopFinding();
      setMatches(undefined);
    }
  }
  function stopFinding() {
    setQuery(undefined);
    setMatches(undefined);
    viewer.current?.stopFinding();
    container.current?.focus();
  }
  function goTo() {
    const page = Number(typed);
    if (Number.isInteger(page)) viewer.current?.goTo(page);
    setTyped(undefined);
  }

  const tools = state && !covered && (
    <>
      <span
        className="pdf-page"
        style={{ '--digits': String(state.pages).length } as CSSProperties}
      >
        <input
          aria-label="Page"
          inputMode="numeric"
          value={typed ?? String(state.page)}
          onFocus={(event) => event.currentTarget.select()}
          onChange={(event) => setTyped(event.target.value)}
          onBlur={() => setTyped(undefined)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') goTo();
            else if (event.key === 'Escape') setTyped(undefined);
          }}
        />
        <span>/ {state.pages}</span>
      </span>
      {!touch && (
        <Button
          size="icon"
          variant="ghost"
          aria-label="Zoom out"
          title="Zoom out"
          onClick={() => viewer.current?.zoom(-1)}
        >
          <ZoomOut size={15} />
        </Button>
      )}
      <DropdownMenu.Root modal={false}>
        <DropdownMenu.Trigger asChild>
          <Button size="sm" variant="ghost" className="pdf-zoom" aria-label="Zoom" title="Zoom">
            {Math.round(state.scale * 100)}%
          </Button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content
            className="menu-panel pdf-zoom-menu"
            align="end"
            sideOffset={4}
            collisionPadding={collisionPadding}
          >
            <DropdownMenu.RadioGroup
              value={state.fit ?? ''}
              onValueChange={(fit) => viewer.current?.fit(fit as Fit)}
            >
              {FIT_NAMES.map(([fit, name]) => (
                <DropdownMenu.RadioItem key={fit} className="menu-item" value={fit}>
                  {name}
                  <DropdownMenu.ItemIndicator className="menu-item-hint">
                    <Check size={14} />
                  </DropdownMenu.ItemIndicator>
                </DropdownMenu.RadioItem>
              ))}
            </DropdownMenu.RadioGroup>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
      {!touch && (
        <Button
          size="icon"
          variant="ghost"
          aria-label="Zoom in"
          title="Zoom in"
          onClick={() => viewer.current?.zoom(1)}
        >
          <ZoomIn size={15} />
        </Button>
      )}
      <Button
        size="icon"
        variant="ghost"
        aria-label="Find"
        aria-pressed={finding}
        title={`Find (${appleKeyboard() ? '⌘F' : 'Ctrl+F'})`}
        onClick={() => (finding ? stopFinding() : find(''))}
      >
        <Search size={15} />
      </Button>
    </>
  );

  return (
    <>
      {bar(tools)}
      <div ref={host} className="pdf-view">
        {query !== undefined && (
          <FindBar
            field={findField}
            className="editor-find"
            label="Find in the PDF"
            query={query}
            count={
              matches?.total
                ? `${matches.current} of ${matches.total}`
                : query && matches && !matches.searching
                  ? 'No matches'
                  : ''
            }
            found={Boolean(matches?.total)}
            onQuery={(text) => find(text)}
            onStep={(step) => query && find(query, step === 1 ? 'next' : 'previous')}
            onStop={stopFinding}
          />
        )}
        <div className="pdf-content">
          <div ref={container} className="pdf-scroll" tabIndex={0} hidden={covered}>
            <div className="pdfViewer" />
          </div>
          {!state && !covered && <Loading label={`Opening ${file.name}…`} fill />}
          {asking && (
            <form
              className="code-empty pdf-password"
              onSubmit={(event) => {
                event.preventDefault();
                asking.answer(password);
              }}
            >
              <Field label="Password" error={asking.wrong ? 'Wrong password.' : undefined}>
                <input
                  type="password"
                  value={password}
                  autoFocus
                  onChange={(event) => setPassword(event.target.value)}
                />
              </Field>
              <Button type="submit" size="sm">
                Open
              </Button>
            </form>
          )}
          {error !== undefined && (
            <div className="code-empty">
              <ErrorNotice error={error} />
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  setError(undefined);
                  setState(undefined);
                  setTries((count) => count + 1);
                }}
              >
                <RotateCw size={14} />
                Try again
              </Button>
            </div>
          )}
        </div>
      </div>
    </>
  );
}
