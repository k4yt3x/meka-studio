import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { useInfiniteQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query';
import {
  ChevronRight,
  FileText,
  FolderGit2,
  List,
  ListTree,
  RefreshCw,
  TriangleAlert,
} from 'lucide-react';
import type { Gateway } from '../api/gateway';
import { useConnection, useGateway, useRuntime, useSettings } from '../connections/context';
import { useAction } from '../components/actions';
import { Empty, ErrorNotice, Loading } from '../components/common';
import { Button } from '../components/ui/button';
import { useMediaQuery } from '../components/media-query';
import { useDarkTheme } from '../components/theme';
import { ago } from '../components/time';
import { useWorkspace } from '../components/workspace';
import { useEditorStore } from '../editor/use-editor';
import {
  changePath,
  changeRows,
  isFolderChange,
  pathspecsOf,
  stagedOf,
  type ChangeRow,
} from '../files/changes-tree';
import { GIT_STATUS, LETTERS, markWords } from '../files/git';
import { WriteQueue } from '../files/git-writes';
import { fileIconName, folderIconName, iconUrl } from '../files/icons';
import { basename, join, parent, within } from '../files/paths';
import { useFolderWatches } from '../files/watch';
import { treeMeasures } from './file-tree-measures';
import { Branch } from './git-branch';
import { useGitRefresh } from './git-status';
import { LineCounts } from './line-counts';

type Change = Gateway['Change'];
type FileStat = Gateway['FileStat'];

const DIFF_STAT = '/api/git/diffstat';
const HISTORY = '/api/git/history';
const COMMIT = '/api/git/commit';

/** The most rows listed; past them, how many more changes there are is said. */
const MAX_LISTED = 1000;
/** The most folders of changed files followed for changes, beside the repository's own. */
const MAX_FOLLOWED = 16;

/**
 * The repository the working directory `start` is in, as git says, in the session `place`: what
 * changed since the last commit, to compare, stage and commit, and the history of its branch. The
 * changes follow the repository's folder, its git folder, the folders of the files listed and of
 * those open, and each tool call the agent finishes.
 */
export function GitPanel({ place, start }: { place: string; start: string }) {
  const listing = useGateway<Gateway['Listing']>('/api/files/list', { path: start });
  const repo = listing.data?.repo ?? undefined;
  const status = useGateway<Gateway['GitStatus']>(
    GIT_STATUS,
    { repo: repo ?? '' },
    repo !== undefined,
  );
  const [view, setView] = useState<'changes' | 'history'>('changes');
  const writes = useGitWrites(repo ?? '', status);
  if (listing.error) return <ErrorNotice error={listing.error} />;
  if (listing.isPending) return <Loading fill />;
  if (repo === undefined) return <Empty title="No git repository" />;
  const shown = writes.status;
  const count = shown?.changes.filter((change) => change.kind !== 'ignored').length;
  return (
    <div className="file-browser changes">
      <div className="changes-views" role="group" aria-label="Git views">
        <Button
          size="sm"
          variant={view === 'changes' ? 'secondary' : 'ghost'}
          aria-pressed={view === 'changes'}
          onClick={() => setView('changes')}
        >
          Changes
          {count !== undefined && count > 0 && (
            <span className="changes-count">{count.toLocaleString()}</span>
          )}
        </Button>
        <Button
          size="sm"
          variant={view === 'history' ? 'secondary' : 'ghost'}
          aria-pressed={view === 'history'}
          onClick={() => setView('history')}
        >
          History
        </Button>
      </div>
      <ErrorNotice error={status.error} />
      {status.isPending ? (
        <Loading fill />
      ) : !shown ? null : view === 'changes' ? (
        <ChangesList
          place={place}
          repo={repo}
          status={shown}
          read={status.dataUpdatedAt}
          writes={writes}
        />
      ) : (
        <HistoryList repo={repo} head={shown.commit ?? undefined} />
      )}
      {shown && view === 'changes' && <CommitBox status={shown} writes={writes} />}
    </div>
  );
}

/**
 * Staging and committing in `repo`, one at a time in the order asked. What a change is staged as
 * shows at once, over what `status` says, until the status read after the last write says it.
 */
function useGitWrites(repo: string, status: UseQueryResult<Gateway['GitStatus']>) {
  const runtime = useRuntime();
  const queries = useQueryClient();
  const device = useConnection().account?.device_id;
  const [staging, setStaging] = useState<ReadonlyMap<string, Gateway['Staged']>>(new Map());
  const [failure, setFailure] = useState<unknown>();
  const refetch = status.refetch;
  /** Reads again how `repo` stands, its changes' lines and its history. */
  const refresh = async () => {
    for (const path of [DIFF_STAT, HISTORY])
      void queries.invalidateQueries({ queryKey: ['gateway', device, path, { repo }] });
    await refetch();
  };
  // Once the last write is done, all is read again.
  const settle = useRef(refresh);
  const [queue] = useState(
    () =>
      new WriteQueue(async () => {
        try {
          await settle.current();
        } finally {
          // A write asked for meanwhile shows on, until the status read after it.
          if (!queue.busy) setStaging(new Map());
        }
      }),
  );
  useEffect(() => {
    settle.current = refresh;
  });
  const send = (path: string, body: object) =>
    queue.run(() => runtime.gateway.send('POST', path, body));
  const data = status.data;
  return {
    /** How `repo` stands, with what is being staged or unstaged shown so already. */
    status:
      data && staging.size
        ? {
            ...data,
            changes: data.changes.map((change) => {
              const staged = staging.get(change.path);
              return staged ? { ...change, staged } : change;
            }),
          }
        : data,
    /** Why staging or unstaging last failed. */
    failure,
    dismiss: () => setFailure(undefined),
    refresh: () => void refresh(),
    /** Stages `changes`, or with `staged` false, unstages them, by `paths`, theirs unless given. */
    stage(changes: readonly Change[], staged: boolean, paths = pathspecsOf(changes)) {
      setFailure(undefined);
      setStaging(
        (before) =>
          new Map([
            ...before,
            ...changes.map((change) => [change.path, staged ? 'all' : 'none'] as const),
          ]),
      );
      send(staged ? '/api/git/stage' : '/api/git/unstage', { repo, paths }).catch(setFailure);
    },
    /** Commits what is staged, once what is being staged is; with `tracked`, every tracked change. */
    commit: (message: string, tracked: boolean) =>
      send('/api/git/commit', { repo, message, tracked }),
  };
}

type GitWrites = ReturnType<typeof useGitWrites>;

/** What changed since the last commit, as a tree or by name, each with its lines and its staging. */
function ChangesList({
  place,
  repo,
  status,
  read,
  writes,
}: {
  place: string;
  repo: string;
  status: Gateway['GitStatus'];
  /** When the status was read, which what is being staged does not change. */
  read: number;
  writes: GitWrites;
}) {
  const workspace = useWorkspace();
  const store = useEditorStore();
  const runtime = useRuntime();
  const dark = useDarkTheme();
  const coarse = useMediaQuery('(pointer: coarse)');
  const { filesFontSize, changesTree } = useSettings();
  const [closed, setClosed] = useState<ReadonlySet<string>>(new Set());
  const lines = useGateway<Gateway['DiffStat']>(DIFF_STAT, { repo });
  // Counted again whenever the status is read again.
  const counted = useRef(read);
  const recount = lines.refetch;
  useEffect(() => {
    if (counted.current === read) return;
    counted.current = read;
    void recount();
  }, [read, recount]);
  const stats = new Map((lines.data?.files ?? []).map((file) => [file.path, file]));
  const changes = status.changes.filter((change) => change.kind !== 'ignored');
  const refresh = useGitRefresh(place);
  const gitDir = status.git_dir ?? undefined;
  const followed = [
    ...new Set([
      repo,
      ...(gitDir ? [gitDir] : []),
      ...changes
        .filter((change) => !isFolderChange(change) && change.kind !== 'deleted')
        .map((change) => parent(join(repo, change.path)))
        .slice(0, MAX_FOLLOWED),
      ...store.folders(place).filter((folder) => within(folder, repo)),
    ]),
  ];
  useFolderWatches(followed, () => refresh(repo));
  const everything = stagedOf(changes);
  const total = (lines.data?.files ?? []).reduce<[number, number]>(
    (sum, file) => [sum[0] + (file.added ?? 0), sum[1] + (file.removed ?? 0)],
    [0, 0],
  );
  const active = store.tabs(place).active;
  const shown = active !== undefined && store.changesOf(active) !== undefined ? active : undefined;
  const rows = changeRows(changes, { tree: changesTree, closed });
  const listed = rows.slice(0, MAX_LISTED);
  const toggle = (path: string) =>
    setClosed((before) => {
      const after = new Set(before);
      if (!after.delete(path)) after.add(path);
      return after;
    });
  return (
    <>
      <div className="file-toolbar" role="toolbar" aria-label="Changes">
        <span className="changes-repo" title={repo}>
          <FolderGit2 size={14} aria-hidden="true" />
          <span>{basename(repo)}</span>
        </span>
        {changes.length > 0 && lines.data && !lines.data.too_many && (
          <LineCounts added={total[0]} removed={total[1]} />
        )}
        <span className="file-toolbar-gap" />
        <Button
          variant="ghost"
          size="icon"
          aria-label={changesTree ? 'List by name' : 'List as a tree'}
          title={changesTree ? 'List by name' : 'List as a tree'}
          onClick={() => runtime.storage.changesTree(!changesTree)}
        >
          {changesTree ? <List size={16} /> : <ListTree size={16} />}
        </Button>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Refresh"
          title="Refresh"
          onClick={writes.refresh}
        >
          <RefreshCw size={16} />
        </Button>
        {changes.length > 0 && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => writes.stage(changes, everything !== 'all', ['.'])}
          >
            {everything === 'all' ? 'Unstage all' : 'Stage all'}
          </Button>
        )}
      </div>
      <ErrorNotice error={writes.failure} onDismiss={writes.dismiss} />
      <ErrorNotice error={lines.error} />
      {status.too_many ? (
        <p className="changes-note">
          <TriangleAlert size={14} aria-hidden="true" />
          Too many changes to list.
        </p>
      ) : !changes.length ? (
        <Empty title="No changes" />
      ) : (
        <ul
          className="file-tree changes-list"
          aria-label="Changed files"
          style={treeMeasures(filesFontSize, coarse).style}
        >
          {listed.map((row) =>
            row.kind === 'folder' ? (
              <FolderRow
                key={`folder:${row.path}`}
                row={row}
                open={!closed.has(row.path)}
                dark={dark}
                onToggle={() => toggle(row.path)}
                onStage={(staged) => writes.stage(row.changes, staged)}
              />
            ) : (
              <ChangeEntry
                // A file deleted from git but left on disk is told twice: deleted, and new.
                key={`${row.change.kind}:${row.change.path}`}
                row={row}
                flat={!changesTree}
                repo={repo}
                stat={stats.get(changePath(row.change))}
                dark={dark}
                current={join(repo, changePath(row.change)) === shown}
                onStage={(staged) => writes.stage([row.change], staged)}
                onCompare={(path, how) =>
                  workspace
                    ? workspace.openChanges(path, repo, how)
                    : store.openChanges(place, path, repo, how)
                }
                onOpen={(path) =>
                  workspace
                    ? workspace.openFile(path, { preview: true })
                    : store.open(place, path, { preview: true })
                }
                onShow={(path) => workspace?.showFile(path)}
              />
            ),
          )}
          {rows.length > listed.length && (
            <li className="changes-more muted">
              {(rows.length - listed.length).toLocaleString()} more not listed
            </li>
          )}
        </ul>
      )}
    </>
  );
}

/** A checkbox that says how much of something is staged, and stages or unstages it all. */
function StagedBox({
  staged,
  label,
  onStage,
}: {
  staged: Gateway['Staged'];
  label: string;
  onStage: (staged: boolean) => void;
}) {
  return (
    <input
      type="checkbox"
      className="file-select"
      aria-label={label}
      title={staged === 'all' ? 'Unstage' : 'Stage'}
      checked={staged === 'all'}
      ref={(element) => {
        if (element) element.indeterminate = staged === 'some';
      }}
      onChange={() => onStage(staged !== 'all')}
    />
  );
}

/** A folder holding changes, which opens and closes, and stages or unstages all it holds. */
function FolderRow({
  row,
  open,
  dark,
  onToggle,
  onStage,
}: {
  row: Extract<ChangeRow, { kind: 'folder' }>;
  open: boolean;
  dark: boolean;
  onToggle: () => void;
  onStage: (staged: boolean) => void;
}) {
  return (
    <li className="file-tree-row changes-row" style={{ '--depth': row.depth } as CSSProperties}>
      <button
        type="button"
        className="changes-entry"
        aria-expanded={open}
        title={row.path}
        onClick={onToggle}
      >
        <span className="file-tree-twisty" aria-hidden="true">
          <ChevronRight />
        </span>
        <img
          className="file-tree-icon"
          src={iconUrl(folderIconName(basename(row.path), open), dark)}
          alt=""
          draggable={false}
        />
        <span className="file-tree-name">{row.name}</span>
      </button>
      <span className="changes-gap" />
      <StagedBox staged={stagedOf(row.changes)} label={`Stage ${row.name}`} onStage={onStage} />
    </li>
  );
}

/**
 * A change: the entry's name, and in a flat list the folder it is in, in the colour of its change,
 * with its lines and its letter; a file opens beside the last commit, or alone with the button
 * beside it, and a folder or a submodule shows in Files.
 */
function ChangeEntry({
  row,
  flat,
  repo,
  stat,
  dark,
  current,
  onStage,
  onCompare,
  onOpen,
  onShow,
}: {
  row: Extract<ChangeRow, { kind: 'change' }>;
  flat: boolean;
  repo: string;
  stat: FileStat | undefined;
  dark: boolean;
  /** Whether the editor shows it beside the last commit. */
  current: boolean;
  onStage: (staged: boolean) => void;
  /** Opens the file at `path` beside the last commit: to look at, with `preview`, or to keep. */
  onCompare: (path: string, how: { preview: boolean }) => void;
  onOpen: (path: string) => void;
  onShow: (path: string) => void;
}) {
  const { change, name } = row;
  const folder = isFolderChange(change);
  const path = join(repo, changePath(change));
  const words = markWords({ kind: change.kind, own: true }, folder);
  return (
    <li
      className="file-tree-row changes-row"
      style={{ '--depth': row.depth } as CSSProperties}
      data-git={change.kind}
      data-current={current || undefined}
    >
      <button
        type="button"
        className="changes-entry"
        aria-current={current || undefined}
        title={`${changePath(change)}\n${change.from ? `Renamed from ${change.from}` : words}${folder ? '\nShow in Files' : ''}`}
        onClick={() => (folder ? onShow(path) : onCompare(path, { preview: true }))}
        onDoubleClick={() => {
          if (!folder) onCompare(path, { preview: false });
        }}
      >
        {/* Under a folder's arrow, as Files' entries are. */}
        {!flat && <span className="file-tree-twisty" aria-hidden="true" />}
        <img
          className="file-tree-icon"
          src={iconUrl(folder ? folderIconName(name, false) : fileIconName(name), dark)}
          alt=""
          draggable={false}
        />
        <span className="file-tree-name">
          {name}
          {flat && row.folder && <span className="changes-folder">{row.folder}</span>}
        </span>
        {stat?.added != null && stat.removed != null && (
          <LineCounts added={stat.added} removed={stat.removed} />
        )}
        <span className="file-tree-git" aria-label={words}>
          {LETTERS[change.kind]}
        </span>
      </button>
      {!folder && change.kind !== 'deleted' ? (
        <Button
          variant="ghost"
          size="icon"
          className="file-tree-more"
          aria-label={`Open ${name}`}
          title="Open file"
          onClick={() => onOpen(path)}
        >
          <FileText size={14} />
        </Button>
      ) : (
        // Its letter lines up with those of the rows that have the button.
        <span className="changes-gap" />
      )}
      <StagedBox staged={change.staged} label={`Stage ${name}`} onStage={onStage} />
    </li>
  );
}

/**
 * The message to commit with, and the branch: what is staged commits, once what is being staged
 * is, or with nothing staged, every change to a file git already tracks, as `git commit -a`
 * does. Ctrl+Enter commits.
 */
function CommitBox({ status, writes }: { status: Gateway['GitStatus']; writes: GitWrites }) {
  const action = useAction();
  const [message, setMessage] = useState('');
  const changes = status.changes.filter((change) => change.kind !== 'ignored');
  const staged = changes.some((change) => change.staged !== 'none');
  const tracked = changes.some((change) => change.kind !== 'untracked');
  const can = (staged || tracked) && message.trim() !== '' && !action.busy;
  const commit = () => {
    if (!can) return;
    void action.run(async () => {
      await writes.commit(message, !staged);
      setMessage('');
    });
  };
  return (
    <div className="changes-commit">
      <ErrorNotice error={action.error} onDismiss={action.reset} />
      <textarea
        className="changes-message"
        aria-label="Commit message"
        placeholder="Commit message"
        rows={3}
        value={message}
        onChange={(event) => setMessage(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            commit();
          }
        }}
      />
      <div className="changes-commit-bar">
        <Branch status={status} />
        <Button
          size="sm"
          disabled={!can}
          aria-busy={action.busy}
          title={staged ? 'Commit what is staged' : 'Commit every change to a tracked file'}
          onClick={commit}
        >
          {action.busy ? 'Committing…' : staged || !tracked ? 'Commit' : 'Commit tracked'}
        </Button>
      </div>
    </div>
  );
}

/** The branch's commits, newest first, more as the list's end comes into view. */
function HistoryList({ repo, head }: { repo: string; head: string | undefined }) {
  const runtime = useRuntime();
  const coarse = useMediaQuery('(pointer: coarse)');
  const { filesFontSize } = useSettings();
  const device = useConnection().account?.device_id;
  const history = useInfiniteQuery({
    queryKey: ['gateway', device, HISTORY, { repo, head }],
    initialPageParam: 0,
    queryFn: ({ pageParam, signal }) =>
      runtime.gateway.get<Gateway['History']>(
        HISTORY,
        { repo, head: head ?? '', skip: pageParam },
        signal,
      ),
    getNextPageParam: (page, pages) =>
      page.more ? pages.reduce((sum, each) => sum + each.commits.length, 0) : undefined,
    enabled: head !== undefined,
  });
  const end = useRef<HTMLLIElement>(null);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = history;
  useEffect(() => {
    const element = end.current;
    if (!element || !hasNextPage) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting) && !isFetchingNextPage)
        void fetchNextPage();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);
  const [open, setOpen] = useState<string>();
  if (head === undefined) return <Empty title="No commits yet" />;
  if (!history.data)
    return history.error ? <ErrorNotice error={history.error} /> : <Loading fill />;
  const commits = history.data.pages.flatMap((page) => page.commits);
  return (
    <>
      <ErrorNotice error={history.error} />
      <ul
        className="changes-history"
        aria-label="Commits"
        style={treeMeasures(filesFontSize, coarse).style}
      >
        {commits.map((commit) => (
          <CommitEntry
            key={commit.id}
            repo={repo}
            commit={commit}
            open={open === commit.id}
            onToggle={() => setOpen(open === commit.id ? undefined : commit.id)}
          />
        ))}
        {hasNextPage && !history.error && (
          <li ref={end} className="changes-more muted">
            Loading more…
          </li>
        )}
      </ul>
    </>
  );
}

/** A commit: its subject, author and when; open, the rest of its message and what it changed. */
function CommitEntry({
  repo,
  commit,
  open,
  onToggle,
}: {
  repo: string;
  commit: Gateway['Commit'];
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <li className="changes-commit-entry">
      <button
        type="button"
        className="changes-commit-head"
        aria-expanded={open}
        title={commit.id}
        onClick={onToggle}
      >
        <span className="changes-commit-subject">{commit.subject}</span>
        <span className="changes-commit-meta muted">
          {commit.author} · {ago(commit.date) ?? commit.date} · {commit.id.slice(0, 7)}
        </span>
      </button>
      {open && <CommitFiles repo={repo} commit={commit} />}
    </li>
  );
}

/** What a commit changed, each file opening beside what the commit before it held. */
function CommitFiles({ repo, commit }: { repo: string; commit: Gateway['Commit'] }) {
  const workspace = useWorkspace();
  const dark = useDarkTheme();
  const coarse = useMediaQuery('(pointer: coarse)');
  const { filesFontSize } = useSettings();
  const changes = useGateway<Gateway['CommitChanges']>(COMMIT, { repo, commit: commit.id });
  if (changes.error) return <ErrorNotice error={changes.error} />;
  if (!changes.data) return <Loading />;
  const body = changes.data.message.split('\n').slice(1).join('\n').trim();
  return (
    <div className="changes-commit-files">
      {body && <p className="changes-commit-body">{body}</p>}
      {changes.data.too_many ? (
        <p className="changes-note">
          <TriangleAlert size={14} aria-hidden="true" />
          Too many changes to list.
        </p>
      ) : (
        <ul
          className="file-tree changes-list"
          aria-label="Files changed"
          style={treeMeasures(filesFontSize, coarse).style}
        >
          {changes.data.files.map((file) => {
            const slash = file.path.lastIndexOf('/');
            const name = file.path.slice(slash + 1);
            const words = markWords({ kind: file.kind, own: true }, false);
            const opened = {
              repo,
              commit: commit.id,
              parent: commit.parents[0] ?? null,
              path: file.path,
              from: file.from ?? null,
            };
            return (
              <li key={file.path} className="file-tree-row changes-row" data-git={file.kind}>
                <button
                  type="button"
                  className="changes-entry"
                  title={`${file.path}\n${file.from ? `Renamed from ${file.from}` : words}`}
                  onClick={() => workspace?.openCommitFile(opened, { preview: true })}
                  onDoubleClick={() => workspace?.openCommitFile(opened, { preview: false })}
                >
                  <img
                    className="file-tree-icon"
                    src={iconUrl(fileIconName(name), dark)}
                    alt=""
                    draggable={false}
                  />
                  <span className="file-tree-name">
                    {name}
                    {slash > 0 && (
                      <span className="changes-folder">{file.path.slice(0, slash)}</span>
                    )}
                  </span>
                  {file.added != null && file.removed != null && (
                    <LineCounts added={file.added} removed={file.removed} />
                  )}
                  <span className="file-tree-git" aria-label={words}>
                    {LETTERS[file.kind]}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
