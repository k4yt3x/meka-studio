/**
 * Where `query` is in `text`, whatever their letters' case: each match's start and end, in UTF-16
 * code units, at most `limit` of them, none overlapping. An empty query is found nowhere.
 */
export function matchesIn(text: string, query: string, limit: number): [number, number][] {
  if (!query) return [];
  const pattern = new RegExp(query.replace(/[.*+?^${}()|[\]\\/]/g, '\\$&'), 'giu');
  const found: [number, number][] = [];
  for (const match of text.matchAll(pattern)) {
    if (found.length === limit) break;
    found.push([match.index, match.index + match[0].length]);
  }
  return found;
}

/** The index, in `starts`, ascending, of the last that is at or before `offset`. */
export function pieceAt(starts: readonly number[], offset: number) {
  let low = 0;
  let high = starts.length - 1;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (starts[middle]! <= offset) low = middle;
    else high = middle - 1;
  }
  return low;
}
