import { describe, expect, it } from 'vitest';
import { joinArgs, splitArgs } from './shell-args';

describe('splitArgs', () => {
  it('separates at white space and keeps quoted parts together', () => {
    expect(splitArgs('  -d   Ubuntu ')).toEqual(['-d', 'Ubuntu']);
    expect(splitArgs(`-Command "Write-Host 'hi there'"`)).toEqual([
      '-Command',
      "Write-Host 'hi there'",
    ]);
    expect(splitArgs(`a"b c"'d'`)).toEqual(['ab cd']);
    expect(splitArgs('"" x')).toEqual(['', 'x']);
    expect(splitArgs('')).toEqual([]);
  });

  it('keeps backslashes, and runs an open quote to the end', () => {
    expect(splitArgs(String.raw`-File C:\Users\me\profile.ps1`)).toEqual([
      '-File',
      String.raw`C:\Users\me\profile.ps1`,
    ]);
    expect(splitArgs('-c "echo hi')).toEqual(['-c', 'echo hi']);
  });
});

describe('joinArgs', () => {
  it('writes arguments that split back the same', () => {
    const samples = [
      ['-d', 'Ubuntu 24.04'],
      ['', 'plain', String.raw`C:\Program Files\x`],
      [`say "hi"`, "it's", `both "and" 'kinds'`, `"`, `'`],
    ];
    for (const args of samples) expect(splitArgs(joinArgs(args))).toEqual(args);
    expect(joinArgs(['-NoLogo', '-d'])).toBe('-NoLogo -d');
  });
});
