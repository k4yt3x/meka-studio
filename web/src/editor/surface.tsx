import { lazy, Suspense, type RefObject } from 'react';
import { Loading } from '../components/common';
import type { EditorStore, OpenFile } from './store';

/** What an editor takes: the file to show, kept in the store's documents. */
export interface SurfaceProps {
  store: EditorStore;
  file: OpenFile;
  /** How many times the file was asked for; each time, it goes to the lines it was opened at. */
  asked: number;
  readOnly: boolean;
  dark: boolean;
  fontSize: number;
  /** Whether long lines wrap; a touch screen's editor always wraps them. */
  wrap: boolean;
  /** Told of Ctrl with `=`, `-` or `0`, or the wheel: a step up or down, or `0` for the saved size. */
  onZoom: (step: -1 | 0 | 1) => void;
  /** Told of Alt+Z, which switches wrapping. */
  onWrap: () => void;
  onSave: () => void;
  onCursor: (line: number, column: number) => void;
  /** Told the language the file is highlighted as, by its name, once that is known. */
  onLanguage: (name: string) => void;
  /** Told how the file indents, as "Spaces: 4" or "Tab size: 4", where the editor knows. */
  onIndentation: (text: string) => void;
}

/** What a comparison shows on its right: an open file, still to edit unless read only, or text. */
export type Compared = { store: EditorStore; file: OpenFile } | { text: string };

/** Moves through a comparison's changes, while it shows. */
export interface CompareControls {
  next(): void;
  previous(): void;
}

/** What comparing text with other text takes: the other on the left, see {@link Compared}. */
export interface CompareProps {
  /** The file's name, which says how the two are highlighted. */
  name: string;
  original: string;
  modified: Compared;
  readOnly: boolean;
  dark: boolean;
  fontSize: number;
  wrap: boolean;
  /** Whether the two show side by side where there is room, rather than as one column. */
  sideBySide: boolean;
  onZoom: (step: -1 | 0 | 1) => void;
  onSave?: (() => void) | undefined;
  /** Given how to move through the changes, while it shows. */
  controlsRef?: RefObject<CompareControls | null> | undefined;
  /** Told how many lines the right adds and removes, as that changes. */
  onStats?: ((added: number, removed: number) => void) | undefined;
}

// Each is loaded the first time a file opens: Monaco where there is a mouse, CodeMirror on a touch
// screen, where Monaco cannot select or type well.
const Monaco = lazy(() =>
  import('./monaco-surface').then((module) => ({ default: module.Surface })),
);
const MonacoCompare = lazy(() =>
  import('./monaco-surface').then((module) => ({ default: module.Compare })),
);
const Touch = lazy(() =>
  import('./codemirror-surface').then((module) => ({ default: module.Surface })),
);
const TouchCompare = lazy(() =>
  import('./codemirror-surface').then((module) => ({ default: module.Compare })),
);

/** The editor for a text file, as the screen suits: see {@link SurfaceProps}. */
export function EditorSurface({ touch, ...props }: SurfaceProps & { touch: boolean }) {
  return (
    <Suspense fallback={<Loading label="Opening the editor…" fill />}>
      {touch ? <Touch {...props} /> : <Monaco {...props} />}
    </Suspense>
  );
}

/**
 * A file beside other text: see {@link CompareProps}. On a touch screen the two are one, read
 * only, what was taken out over what came in.
 */
export function CompareSurface({ touch, ...props }: CompareProps & { touch: boolean }) {
  return (
    <Suspense fallback={<Loading label="Opening the comparison…" fill />}>
      {touch ? <TouchCompare {...props} /> : <MonacoCompare {...props} />}
    </Suspense>
  );
}
