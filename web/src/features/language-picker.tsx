import * as Popover from '@radix-ui/react-popover';
import { Check } from 'lucide-react';
import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { useMediaQuery } from '../components/media-query';
import { useOverlayPadding } from '../components/visual-viewport';
import type { Language } from '../editor/store';

/** A language to choose, or with no id, the one the file's name says. */
interface Option {
  id: string | undefined;
  name: string;
}

const AUTO: Option = { id: undefined, name: 'Auto detect' };

/**
 * The languages whose name or id holds `query`, those that start with it first; with none typed,
 * every one, after {@link AUTO}.
 */
function matching(languages: readonly Language[], query: string): Option[] {
  const wanted = query.trim().toLowerCase();
  if (!wanted) return [AUTO, ...languages];
  const holds = (text: string, start: boolean) =>
    start ? text.toLowerCase().startsWith(wanted) : text.toLowerCase().includes(wanted);
  const found = languages.filter((each) => holds(each.name, false) || holds(each.id, false));
  const first = found.filter((each) => holds(each.name, true) || holds(each.id, true));
  return [...first, ...found.filter((each) => !first.includes(each))];
}

/**
 * The language a file is highlighted as, in the editor's status line, which opens a list to choose
 * another from, found by typing part of its name.
 */
export function LanguagePicker({
  name,
  chosen,
  languages,
  onChoose,
}: {
  /** The language the file is highlighted as, by its name. */
  name: string;
  /** The language chosen for the file, by its id, if one was. */
  chosen: string | undefined;
  /** The languages to choose from, read as the list opens. */
  languages: () => readonly Language[];
  /** Chooses the language `id`, or with none, the one the file's name says. */
  onChoose: (id: string | undefined) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const listId = useId();
  const list = useRef<HTMLDivElement>(null);
  const collisionPadding = useOverlayPadding();
  const coarse = useMediaQuery('(pointer: coarse)');
  const options = open ? matching(languages(), query) : [];
  // The option the keys are on stays in sight.
  useEffect(() => {
    list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [active, open]);
  function show(next: boolean) {
    setOpen(next);
    if (!next) return;
    // Opened on what is chosen, with nothing typed.
    setQuery('');
    setActive(
      Math.max(
        0,
        matching(languages(), '').findIndex((each) => each.id === chosen),
      ),
    );
  }
  function choose(option: Option | undefined) {
    if (!option) return;
    onChoose(option.id);
    setOpen(false);
  }
  function keyDown(event: KeyboardEvent<HTMLInputElement>) {
    const last = options.length - 1;
    const to =
      event.key === 'ArrowDown'
        ? Math.min(active + 1, last)
        : event.key === 'ArrowUp'
          ? Math.max(active - 1, 0)
          : event.key === 'PageDown'
            ? Math.min(active + 10, last)
            : event.key === 'PageUp'
              ? Math.max(active - 10, 0)
              : undefined;
    if (to !== undefined) {
      event.preventDefault();
      setActive(Math.max(0, to));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      choose(options[active]);
    }
  }
  return (
    <Popover.Root open={open} onOpenChange={show}>
      <Popover.Trigger asChild>
        <button
          type="button"
          className="code-status-language"
          aria-label={`Language: ${name}`}
          title="Select the language"
        >
          {name}
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          className="language-picker"
          aria-label="Language"
          side="top"
          align="end"
          sideOffset={6}
          collisionPadding={collisionPadding}
          onOpenAutoFocus={(event) => {
            // Keep the on-screen keyboard closed until the field is chosen.
            if (coarse) event.preventDefault();
          }}
        >
          <input
            role="combobox"
            aria-label="Search languages"
            aria-expanded="true"
            aria-controls={listId}
            aria-autocomplete="list"
            aria-activedescendant={options.length ? `${listId}-${active}` : undefined}
            placeholder="Search languages"
            value={query}
            autoCapitalize="off"
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            onKeyDown={keyDown}
          />
          <div ref={list} id={listId} role="listbox" aria-label="Languages">
            {options.map((option, index) => (
              <div
                key={option.id ?? ''}
                id={`${listId}-${index}`}
                role="option"
                aria-selected={index === active}
                data-index={index}
                className="language-option"
                onPointerMove={() => setActive(index)}
                onClick={() => choose(option)}
              >
                <span>{option.name}</span>
                {option.id === chosen && <Check size={14} aria-label="Chosen" />}
              </div>
            ))}
            {!options.length && <p className="language-none">No language matches</p>}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
