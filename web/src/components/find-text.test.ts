import { describe, expect, it } from 'vitest';
import { matchesIn, pieceAt } from './find-text';

describe('finding text', () => {
  it('finds every match, whatever the case, and none for an empty query', () => {
    expect(matchesIn('Meka meets MEKA', 'meka', 10)).toEqual([
      [0, 4],
      [11, 15],
    ]);
    expect(matchesIn('anything', '', 10)).toEqual([]);
    expect(matchesIn('aaaa', 'aa', 10)).toEqual([
      [0, 2],
      [2, 4],
    ]);
  });

  it('takes the query as written, not as a pattern', () => {
    expect(matchesIn('a.b axb (c) [d] a/b', 'a.b', 10)).toEqual([[0, 3]]);
    expect(matchesIn('a.b axb (c) [d] a/b', '(c)', 10)).toEqual([[8, 11]]);
    expect(matchesIn('a.b axb (c) [d] a/b', '[d]', 10)).toEqual([[12, 15]]);
    expect(matchesIn('a.b axb (c) [d] a/b', 'a/b', 10)).toEqual([[16, 19]]);
    expect(matchesIn('x-y', '-', 10)).toEqual([[1, 2]]);
  });

  it('stops at the limit', () => {
    expect(matchesIn('a a a a', 'a', 2)).toHaveLength(2);
  });

  it('finds which piece of the text an offset falls in', () => {
    const starts = [0, 5, 5, 12];
    expect(pieceAt(starts, 0)).toBe(0);
    expect(pieceAt(starts, 4)).toBe(0);
    expect(pieceAt(starts, 5)).toBe(2);
    expect(pieceAt(starts, 11)).toBe(2);
    expect(pieceAt(starts, 40)).toBe(3);
  });
});
