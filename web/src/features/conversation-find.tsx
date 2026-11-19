import { useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { FindBar } from '../components/find-bar';
import { matchesIn, pieceAt } from '../components/find-text';
import { useShortcut } from '../components/use-shortcut';

/** The most matches marked, past which the count says there are more. */
const MAX_MATCHES = 1000;

/** How long the conversation is let settle, as a reply streams in, before it is looked in again. */
const SETTLE = 150;

/** What two pieces of text must share to be read as one, so a match does not span paragraphs. */
const BLOCK = 'p, li, pre, td, th, h1, h2, h3, h4, h5, h6, blockquote, summary, dt, dd, article';

/** The names the matches, and the one shown, are drawn under; see `styles.css`. */
const MATCHES = 'conversation-find';
const CURRENT = 'conversation-find-current';

/** The text of what shows under `root`, in pieces, each with where it starts in the whole. */
function shownText(root: HTMLElement) {
  const visible = new Map<Element, boolean>();
  const shown = (element: Element) => {
    let answer = visible.get(element);
    if (answer === undefined) {
      answer = !element.closest('.sr-only') && (element.checkVisibility?.() ?? true);
      visible.set(element, answer);
    }
    return answer;
  };
  const pieces: Text[] = [];
  const starts: number[] = [];
  let text = '';
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const piece = node as Text;
    if (!piece.data || !piece.parentElement || !shown(piece.parentElement)) continue;
    pieces.push(piece);
    starts.push(text.length);
    text += piece.data;
  }
  return { text, pieces, starts };
}

/** Where `query` shows under `root`, as ranges, within one paragraph each, in reading order. */
function find(root: HTMLElement, query: string): Range[] {
  const { text, pieces, starts } = shownText(root);
  return matchesIn(text, query, MAX_MATCHES + 1).flatMap(([start, end]) => {
    const first = pieceAt(starts, start);
    const last = pieceAt(starts, end - 1);
    const from = pieces[first]!;
    const to = pieces[last]!;
    if (from !== to && from.parentElement?.closest(BLOCK) !== to.parentElement?.closest(BLOCK))
      return [];
    const range = document.createRange();
    range.setStart(from, start - starts[first]!);
    range.setEnd(to, end - starts[last]!);
    return [range];
  });
}

/** Scrolls `scroller` to show `range`, in the middle of what the message box leaves in sight. */
function reveal(scroller: HTMLElement, range: Range) {
  const box = range.getBoundingClientRect();
  const view = scroller.getBoundingClientRect();
  const covered = parseFloat(getComputedStyle(scroller).getPropertyValue('--composer-height')) || 0;
  const bottom = view.bottom - covered;
  if (box.top >= view.top && box.bottom <= bottom) return;
  scroller.scrollTop += box.top - (view.top + (bottom - view.top - box.height) / 2);
}

/** The page's registry of highlights, where the webview has one. */
function highlights(): HighlightRegistry | undefined {
  return typeof CSS !== 'undefined' && 'highlights' in CSS ? CSS.highlights : undefined;
}

/**
 * Finding text in the conversation with Ctrl+F (⌘F), in the desktop app, whose webview has no
 * find of its own: in what shows of the messages loaded, the matches marked, Enter and Shift+Enter
 * going to the next and previous, which scroll into sight.
 */
export function ConversationFind({
  scroller,
  content,
}: {
  scroller: RefObject<HTMLElement | null>;
  content: RefObject<HTMLElement | null>;
}) {
  /** What is looked for, while finding. */
  const [query, setQuery] = useState<string>();
  const [ranges, setRanges] = useState<Range[]>([]);
  const [current, setCurrent] = useState(0);
  /** Goes up each time finding is asked for, to select what was typed again. */
  const [asked, setAsked] = useState(0);
  const field = useRef<HTMLInputElement>(null);
  const returnTo = useRef<Element | null>(null);
  const finding = query !== undefined;

  useShortcut('find', () => {
    if (!finding) returnTo.current = document.activeElement;
    // What is selected in the conversation is what is looked for, as browsers do.
    const selected = window.getSelection();
    const words = selected?.toString().trim();
    if (
      words &&
      !words.includes('\n') &&
      words.length <= 200 &&
      selected?.anchorNode &&
      content.current?.contains(selected.anchorNode)
    )
      setQuery(words);
    else setQuery((text) => text ?? '');
    setAsked((count) => count + 1);
  });
  useEffect(() => {
    if (finding) field.current?.select();
  }, [finding, asked]);

  // A new query starts from its first match.
  const [lookedFor, setLookedFor] = useState(query);
  if (lookedFor !== query) {
    setLookedFor(query);
    setCurrent(0);
  }
  // Looked in again as what is typed changes, scrolling to the first match, and as the
  // conversation does, once it settles, staying where it is.
  useEffect(() => {
    const root = content.current;
    if (!root || !query) return;
    const look = () => {
      const found = find(root, query);
      setRanges(found);
      setCurrent((index) => Math.min(index, Math.max(0, found.length - 1)));
      return found;
    };
    const first = look()[0];
    if (first && scroller.current) reveal(scroller.current, first);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const observer = new MutationObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(look, SETTLE);
    });
    observer.observe(root, { childList: true, subtree: true, characterData: true });
    return () => {
      observer.disconnect();
      clearTimeout(timer);
    };
  }, [content, scroller, query]);

  const shown = useMemo(() => (query ? ranges.slice(0, MAX_MATCHES) : []), [query, ranges]);
  const at = shown[current];
  useEffect(() => {
    const registry = highlights();
    if (!registry || !shown.length) return;
    registry.set(MATCHES, new Highlight(...shown));
    if (at) registry.set(CURRENT, new Highlight(at));
    return () => {
      registry.delete(MATCHES);
      registry.delete(CURRENT);
    };
  }, [shown, at]);

  function go(step: -1 | 1) {
    const total = shown.length;
    if (!total) return;
    const next = (current + step + total) % total;
    setCurrent(next);
    const element = scroller.current;
    const range = shown[next];
    if (element && range) reveal(element, range);
  }
  function stop() {
    setQuery(undefined);
    const back = returnTo.current;
    returnTo.current = null;
    if (back instanceof HTMLElement && back.isConnected) back.focus({ preventScroll: true });
    else scroller.current?.focus({ preventScroll: true });
  }

  if (query === undefined) return null;
  const total = ranges.length > MAX_MATCHES ? `${MAX_MATCHES}+` : String(shown.length);
  return (
    <FindBar
      field={field}
      className="conversation-find"
      label="Find in the conversation"
      query={query}
      count={!query ? '' : shown.length ? `${current + 1} of ${total}` : 'No matches'}
      found={shown.length > 0}
      onQuery={setQuery}
      onStep={go}
      onStop={stop}
    />
  );
}
