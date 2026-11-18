import type { Gateway } from '../api/gateway';
import { compareNames } from './tree-model';

type Change = Gateway['Change'];

/** A row of the changes listed: a folder holding some, or a change. */
export type ChangeRow =
  | {
      kind: 'folder';
      /** Its path in the repository, its names separated by `/`. */
      path: string;
      /** Its name, with those of the folders it alone holds after it, as one. */
      name: string;
      depth: number;
      /** The changes it holds, at any depth. */
      changes: Change[];
    }
  | {
      kind: 'change';
      change: Change;
      name: string;
      /** The folder it is in, which a flat list shows beside its name. */
      folder: string;
      depth: number;
    };

/** Whether `change` is of a folder: one new as a whole, or a submodule. */
export function isFolderChange(change: Change): boolean {
  return change.path.endsWith('/') || change.submodule;
}

/** The path of `change` in the repository, without the `/` a folder's ends with. */
export function changePath(change: Change): string {
  return change.path.replace(/\/$/, '');
}

interface Node {
  name: string;
  path: string;
  folders: Map<string, Node>;
  changes: Change[];
}

/**
 * `changes` as listed, those git ignores left out: with `tree`, under the folders holding them,
 * a folder holding one folder alone shown as one with it, folders first and each by name, and
 * nothing within those `closed`; else each by its name, then its folder.
 */
export function changeRows(
  changes: readonly Change[],
  { tree, closed }: { tree: boolean; closed: ReadonlySet<string> },
): ChangeRow[] {
  const shown = changes.filter((change) => change.kind !== 'ignored');
  const split = (change: Change) => {
    const path = changePath(change);
    const slash = path.lastIndexOf('/');
    return { name: path.slice(slash + 1), folder: slash < 0 ? '' : path.slice(0, slash) };
  };
  if (!tree)
    return shown
      .map((change) => ({ kind: 'change' as const, change, depth: 0, ...split(change) }))
      .sort((a, b) => compareNames(a.name, b.name) || compareNames(a.folder, b.folder));
  const root: Node = { name: '', path: '', folders: new Map(), changes: [] };
  for (const change of shown) {
    let node = root;
    const { folder } = split(change);
    for (const name of folder ? folder.split('/') : []) {
      const path = node.path ? `${node.path}/${name}` : name;
      let next = node.folders.get(name);
      if (!next) node.folders.set(name, (next = { name, path, folders: new Map(), changes: [] }));
      node = next;
    }
    node.changes.push(change);
  }
  const rows: ChangeRow[] = [];
  const held = (node: Node): Change[] => [
    ...node.changes,
    ...[...node.folders.values()].flatMap(held),
  ];
  const visit = (node: Node, depth: number) => {
    const entries = [
      ...[...node.folders.values()].map((folder) => {
        // A folder holding only a folder is shown as one with it.
        let name = folder.name;
        let inner = folder;
        while (!inner.changes.length && inner.folders.size === 1) {
          inner = [...inner.folders.values()][0]!;
          name = `${name}/${inner.name}`;
        }
        return { name, folder: true, inner, change: undefined };
      }),
      ...node.changes.map((change) => ({
        name: split(change).name,
        folder: isFolderChange(change),
        inner: undefined,
        change,
      })),
    ].sort((a, b) => (a.folder !== b.folder ? (a.folder ? -1 : 1) : compareNames(a.name, b.name)));
    for (const entry of entries) {
      if (entry.inner) {
        rows.push({
          kind: 'folder',
          path: entry.inner.path,
          name: entry.name,
          depth,
          changes: held(entry.inner),
        });
        if (!closed.has(entry.inner.path)) visit(entry.inner, depth + 1);
      } else if (entry.change)
        rows.push({
          kind: 'change',
          change: entry.change,
          name: entry.name,
          folder: split(entry.change).folder,
          depth,
        });
    }
  };
  visit(root, 0);
  return rows;
}

/** How much of `changes` is staged, together: none, all, or some. */
export function stagedOf(changes: readonly Change[]): Gateway['Staged'] {
  if (changes.every((change) => change.staged === 'all')) return 'all';
  if (changes.every((change) => change.staged === 'none')) return 'none';
  return 'some';
}

/**
 * The paths git takes to stage or unstage `changes`: each change's, a folder's ending in `/`, and
 * where a renamed one was, as its old path is part of its change.
 */
export function pathspecsOf(changes: readonly Change[]): string[] {
  return [
    ...new Set(
      changes.flatMap((change) => (change.from ? [change.path, change.from] : [change.path])),
    ),
  ];
}
