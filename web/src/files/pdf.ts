/**
 * PDF.js, loaded with the first PDF shown: its worker, its viewer's styles, and a viewer of one
 * document in an element, which renders the pages as they scroll into sight. Scripts in a PDF never
 * run, and its forms show as printed. Its build for older browsers is the one taken: the other uses
 * what only the latest have, which the desktop app's WebKit and Safari on phones may not.
 */
import {
  AnnotationEditorType,
  AnnotationMode,
  getDocument,
  GlobalWorkerOptions,
  InvalidPDFException,
  PasswordResponses,
  ResponseException,
  TouchManager,
  type PDFDocumentLoadingTask,
  type PDFDocumentProxy,
} from 'pdfjs-dist/legacy/build/pdf.mjs';
import worker from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url';
import 'pdfjs-dist/legacy/web/pdf_viewer.css';
import { FITS, isFit, type Fit, type PdfPlace } from './pdf-place';

GlobalWorkerOptions.workerSrc = worker;

/** Where a PDF shown stands. */
export interface PdfState {
  page: number;
  pages: number;
  /** The zoom, 1 being the PDF's own size. */
  scale: number;
  /** What the zoom follows, if it follows the room rather than being set. */
  fit: Fit | undefined;
}

/** How a search through a PDF stands: the match shown among those found, and if it goes on. */
export interface Matches {
  current: number;
  total: number;
  searching: boolean;
}

/** A PDF shown in an element. */
export interface PdfViewer {
  /** Zooms a step in or out, about `origin`, a point on the screen, or the middle. */
  zoom(step: -1 | 1, origin?: [number, number]): void;
  fit(fit: Fit): void;
  goTo(page: number): void;
  /** Looks for `query`, from the start or, `again`, on from the match shown. */
  find(query: string, again?: 'next' | 'previous'): void;
  stopFinding(): void;
  /** Where it was read to, to open it there again. */
  place(): PdfPlace | undefined;
  /** Reads the PDF again, as it changed on disk, staying where it was. */
  reload(): void;
  destroy(): void;
}

export interface PdfOptions {
  /** What scrolls: positioned absolutely, holding an empty element for the pages. */
  container: HTMLDivElement;
  url: string;
  /** Where to open it, as {@link PdfViewer.place} told. */
  place: PdfPlace | undefined;
  onState: (state: PdfState) => void;
  onMatches: (matches: Matches) => void;
  /** Asks for the PDF's password, `wrong` after one that was not; it opens with the answer. */
  onPassword: (wrong: boolean) => Promise<string>;
  /** Told that the PDF could not be read. */
  onError: (error: unknown) => void;
}

/** What PDF.js's viewer is made of, loaded once it has put itself where the viewer finds it. */
let parts: Promise<typeof import('pdfjs-dist/legacy/web/pdf_viewer.mjs')> | undefined;

/** How long a zoom scales what shows before drawing it again, as Firefox's reader waits. */
const DRAWING_DELAY = 400;

/** Shows the PDF at `url` in `container`; see {@link PdfOptions}. */
export async function openPdf(options: PdfOptions): Promise<PdfViewer> {
  const { container, url, onState, onMatches, onPassword, onError } = options;
  // One that failed to load, as over a dropped connection, is loaded again for the next PDF.
  parts ??= import('pdfjs-dist/legacy/web/pdf_viewer.mjs').catch((error: unknown) => {
    parts = undefined;
    throw error;
  });
  const { EventBus, FindState, LinkTarget, PDFFindController, PDFLinkService, PDFViewer } =
    await parts;
  const events = new EventBus();
  const links = new PDFLinkService({
    eventBus: events,
    externalLinkTarget: LinkTarget.BLANK,
    externalLinkRel: 'noopener noreferrer nofollow',
  });
  const finder = new PDFFindController({ eventBus: events, linkService: links });
  const viewer = new PDFViewer({
    container,
    eventBus: events,
    linkService: links,
    findController: finder,
    annotationMode: AnnotationMode.ENABLE,
    annotationEditorMode: AnnotationEditorType.DISABLE,
    enableAutoLinking: true,
  });
  links.setViewer(viewer);
  const stopping = new AbortController();
  /** The PDF read last, and the one shown, which it replaces once it opens. */
  let task: PDFDocumentLoadingTask | undefined;
  let shown: PDFDocumentLoadingTask | undefined;
  let place = options.place;
  let location: PdfPlace | undefined;
  let query = '';
  /** The password the PDF opened with, which it is read again with as it changes. */
  let password: string | undefined;

  const state = () =>
    onState({
      page: viewer.currentPageNumber,
      pages: viewer.pagesCount,
      scale: viewer.currentScale,
      fit: FITS.find((fit) => fit === viewer.currentScaleValue),
    });
  events.on('pagesinit', () => {
    if (place) {
      const { page, zoom, left, top } = place;
      if (typeof zoom === 'string') viewer.currentScaleValue = zoom;
      viewer.scrollPageIntoView({
        pageNumber: Math.min(page, viewer.pagesCount),
        destArray: [null, { name: 'XYZ' }, left, top, typeof zoom === 'number' ? zoom / 100 : null],
        allowNegativeOffset: true,
      });
    } else viewer.currentScaleValue = 'auto';
    if (query) find(query);
    state();
  });
  events.on('pagechanging', state);
  events.on('scalechanging', state);
  events.on('updateviewarea', ({ location: at }: { location: ViewLocation }) => {
    location = {
      page: at.pageNumber,
      zoom: isFit(at.scale) ? at.scale : Number(at.scale),
      left: at.left,
      top: at.top,
    };
  });
  // Counts come as the pages are searched; the search goes on until its state says otherwise.
  let searching = false;
  events.on('updatefindcontrolstate', ({ matchesCount, state: found }: Found) => {
    searching = found === FindState.PENDING;
    onMatches({ ...matchesCount, searching });
  });
  events.on('updatefindmatchescount', ({ matchesCount }: Found) =>
    onMatches({ ...matchesCount, searching }),
  );

  // A two-finger pinch zooms the PDF about the fingers, where the page itself is not zoomed.
  new TouchManager({
    container,
    onPinching: (origin: [number, number], before: number, now: number) =>
      viewer.updateScale({ drawingDelay: DRAWING_DELAY, scaleFactor: now / before, origin }),
    signal: stopping.signal,
  });

  async function load() {
    const loading = getDocument({
      url,
      ...(password !== undefined && { password }),
      cMapUrl: `${__PDFJS_FILES__}cmaps/`,
      standardFontDataUrl: `${__PDFJS_FILES__}standard_fonts/`,
      wasmUrl: `${__PDFJS_FILES__}wasm/`,
      // The page's policy refuses WebAssembly; the decoders built to JavaScript stand in.
      useWasm: false,
    });
    task = loading;
    loading.onPassword = (answer: (given: string) => void, reason: number) => {
      void onPassword(reason === PasswordResponses.INCORRECT_PASSWORD).then((given) => {
        password = given;
        answer(given);
      });
    };
    let document: PDFDocumentProxy;
    try {
      document = await loading.promise;
    } catch (error) {
      void loading.destroy();
      if (task === loading && !stopping.signal.aborted) onError(readable(error));
      return;
    }
    if (task !== loading || stopping.signal.aborted) {
      void loading.destroy();
      return;
    }
    const before = shown;
    shown = loading;
    viewer.setDocument(document);
    links.setDocument(document);
    void before?.destroy();
  }

  function find(text: string, again?: 'next' | 'previous') {
    query = text;
    events.dispatch('find', {
      source: null,
      type: again ? 'again' : '',
      query: text,
      caseSensitive: false,
      entireWord: false,
      highlightAll: true,
      findPrevious: again === 'previous',
      matchDiacritics: false,
    });
  }

  void load();
  return {
    zoom: (step, origin) =>
      viewer.updateScale({ drawingDelay: DRAWING_DELAY, steps: step, ...(origin && { origin }) }),
    fit: (fit) => {
      viewer.currentScaleValue = fit;
    },
    goTo: (page) => {
      if (page >= 1 && page <= viewer.pagesCount) viewer.currentPageNumber = page;
    },
    find,
    stopFinding: () => {
      query = '';
      events.dispatch('findbarclose', { source: null });
    },
    place: () => location,
    reload: () => {
      place = location ?? place;
      void load();
    },
    destroy: () => {
      stopping.abort();
      viewer.setDocument(null);
      links.setDocument(null);
      void task?.destroy();
      if (shown !== task) void shown?.destroy();
    },
  };
}

/** `error`, from reading a PDF, as people read it: PDF.js's own messages name its internals. */
function readable(error: unknown): unknown {
  if (error instanceof InvalidPDFException) return new Error('This file is damaged, or not a PDF.');
  if (error instanceof ResponseException)
    return new Error(
      error.status === 404
        ? 'This file is not there.'
        : `This file could not be read (HTTP ${error.status}).`,
    );
  return error;
}

/** Where PDF.js says the view is: the zoom a fit's name, or a percentage. */
interface ViewLocation {
  pageNumber: number;
  scale: number | string;
  left: number;
  top: number;
}

/** How a search stands, as PDF.js tells it. */
interface Found {
  matchesCount: { current: number; total: number };
  state?: number;
}
