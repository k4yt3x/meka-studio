import { expect, it } from 'vitest';
import type { Gateway } from '../api/gateway';
import { changeRows, pathspecsOf, stagedOf } from './changes-tree';

const change = (
  path: string,
  kind: Gateway['ChangeKind'] = 'modified',
  staged: Gateway['Staged'] = 'none',
  from?: string,
): Gateway['Change'] => ({ path, kind, from: from ?? null, submodule: false, staged });

const shown = (rows: ReturnType<typeof changeRows>) =>
  rows.map(
    (row) => `${'  '.repeat(row.depth)}${row.kind === 'folder' ? `${row.name}/` : row.name}`,
  );

const changes = [
  change('crates/meka-studio/src/git.rs'),
  change('crates/meka-studio/src/api/git.rs'),
  change('docs/usage.md'),
  change('README.md'),
  change('fresh/', 'untracked'),
  change('build/', 'ignored'),
  change('a10.txt', 'untracked'),
  change('a9.txt', 'untracked'),
];

it('lists changes under their folders, a folder holding one folder alone as one with it', () => {
  expect(shown(changeRows(changes, { tree: true, closed: new Set() }))).toEqual([
    'crates/meka-studio/src/',
    '  api/',
    '    git.rs',
    '  git.rs',
    'docs/',
    '  usage.md',
    'fresh',
    'a9.txt',
    'a10.txt',
    'README.md',
  ]);
  const folder = changeRows(changes, { tree: true, closed: new Set() })[0];
  expect(folder).toMatchObject({ kind: 'folder', path: 'crates/meka-studio/src' });
  expect(folder?.kind === 'folder' && folder.changes.length).toBe(2);
});

it('leaves out what a closed folder holds', () => {
  expect(
    shown(changeRows(changes, { tree: true, closed: new Set(['crates/meka-studio/src']) })),
  ).toEqual([
    'crates/meka-studio/src/',
    'docs/',
    '  usage.md',
    'fresh',
    'a9.txt',
    'a10.txt',
    'README.md',
  ]);
});

it('lists changes flat by name, then by folder', () => {
  const rows = changeRows(changes, { tree: false, closed: new Set() });
  expect(rows.map((row) => row.kind === 'change' && `${row.name} ${row.folder}`)).toEqual([
    'a9.txt ',
    'a10.txt ',
    'fresh ',
    'git.rs crates/meka-studio/src',
    'git.rs crates/meka-studio/src/api',
    'README.md ',
    'usage.md docs',
  ]);
});

it('says how much of several changes is staged, and stages a rename with where it was', () => {
  expect(stagedOf([change('a', 'modified', 'all'), change('b', 'added', 'all')])).toBe('all');
  expect(stagedOf([change('a'), change('b', 'modified', 'all')])).toBe('some');
  expect(stagedOf([change('a'), change('b', 'modified', 'some')])).toBe('some');
  expect(stagedOf([change('a'), change('b')])).toBe('none');
  expect(
    pathspecsOf([change('new.rs', 'renamed', 'all', 'old.rs'), change('fresh/', 'untracked')]),
  ).toEqual(['new.rs', 'old.rs', 'fresh/']);
});
