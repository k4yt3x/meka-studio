import type { RefObject } from 'react';
import { ChevronDown, ChevronUp, X } from 'lucide-react';
import { Button } from './ui/button';

/**
 * A bar to find text with: what is looked for, where the match shown is among them, and buttons to
 * go to the previous and next match and to stop. In the field, Enter and Shift+Enter go to the next
 * and previous match, and Escape stops.
 */
export function FindBar({
  field,
  className,
  label,
  query,
  count,
  found,
  onQuery,
  onStep,
  onStop,
}: {
  field: RefObject<HTMLInputElement | null>;
  /** Where the bar sits. */
  className: string;
  /** What the field finds text in, as its accessible name. */
  label: string;
  query: string;
  /** Where the match shown is among them, or that there are none; empty while nothing is known. */
  count: string;
  /** Whether there are matches to go to. */
  found: boolean;
  onQuery: (text: string) => void;
  onStep: (step: -1 | 1) => void;
  onStop: () => void;
}) {
  return (
    <div className={`find-bar ${className}`} role="search">
      <input
        ref={field}
        aria-label={label}
        placeholder="Find"
        value={query}
        onChange={(event) => onQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
            event.preventDefault();
            onStep(event.shiftKey ? -1 : 1);
          } else if (event.key === 'Escape') {
            event.preventDefault();
            onStop();
          }
        }}
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
      />
      <span className="find-bar-count" role="status">
        {count}
      </span>
      <Button
        size="icon"
        variant="ghost"
        aria-label="Previous match"
        title="Previous match (Shift+Enter)"
        disabled={!found}
        onClick={() => onStep(-1)}
      >
        <ChevronUp size={15} />
      </Button>
      <Button
        size="icon"
        variant="ghost"
        aria-label="Next match"
        title="Next match (Enter)"
        disabled={!found}
        onClick={() => onStep(1)}
      >
        <ChevronDown size={15} />
      </Button>
      <Button
        size="icon"
        variant="ghost"
        aria-label="Stop finding"
        title="Close (Escape)"
        onClick={onStop}
      >
        <X size={15} />
      </Button>
    </div>
  );
}
