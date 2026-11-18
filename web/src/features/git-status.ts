import { useCallback, useEffect, useRef } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { isProblem, type Gateway } from '../api/gateway';
import { gatewayKey, useConnection, useGateway } from '../connections/context';
import { readText } from '../editor/store';
import { GIT_FILE, GIT_STATUS, gitMarks } from '../files/git';
import { parent } from '../files/paths';
import { useToolsFinished } from '../session/hooks';

/** How long git is waited on after a change, so a burst of them reads its status once. */
const GIT_REFRESH_MS = 500;

/**
 * Reads again, in a moment, the status of the repository it is given, or of every repository
 * with none; and of every one as the tool calls of the session `place` finish, which may have
 * changed files in folders no one follows.
 */
export function useGitRefresh(place: string): (repo?: string) => void {
  const queries = useQueryClient();
  const device = useConnection().account?.device_id;
  const pending = useRef<{ timer?: ReturnType<typeof setTimeout> | undefined; repos: Set<string> }>(
    { repos: new Set() },
  );
  useEffect(() => {
    const current = pending.current;
    return () => clearTimeout(current.timer);
  }, []);
  const refresh = useCallback(
    (repo?: string) => {
      const current = pending.current;
      current.repos.add(repo ?? '');
      if (current.timer !== undefined) return;
      current.timer = setTimeout(() => {
        current.timer = undefined;
        const asked = [...current.repos];
        current.repos.clear();
        for (const each of asked)
          void queries.invalidateQueries({
            queryKey: each
              ? gatewayKey(device, GIT_STATUS, { repo: each })
              : ['gateway', device, GIT_STATUS],
          });
      }, GIT_REFRESH_MS);
    },
    [queries, device],
  );
  const finished = useToolsFinished(place);
  const lastFinished = useRef(finished);
  useEffect(() => {
    if (finished === lastFinished.current) return;
    lastFinished.current = finished;
    if (finished !== undefined) refresh();
  }, [finished, refresh]);
  return refresh;
}

/**
 * What git says of the file `path`: the repository it is in, `known` where it is, else the one
 * its folder's listing names; that repository's status; and how the file changed since the last
 * commit, if it did.
 */
export function useGitFile(path: string, known: string | undefined) {
  const listing = useGateway<Gateway['Listing']>(
    '/api/files/list',
    { path: parent(path) },
    known === undefined,
  );
  const repo = known ?? listing.data?.repo ?? undefined;
  const status = useGateway<Gateway['GitStatus']>(
    GIT_STATUS,
    { repo: repo ?? '' },
    repo !== undefined,
  );
  const change = status.data ? gitMarks(status.data).change(path) : undefined;
  return { repo, status, change };
}

/**
 * The file at `path` in the repository `repo`, as the commit `commit` holds it, to compare: its
 * text, without a byte order mark; empty where there is no commit, or the commit has no file
 * there; none while it is read, or where it is not text or too large to compare, as said.
 */
export function useCommittedText(
  repo: string,
  commit: string | undefined,
  path: string | undefined,
) {
  const read = useGateway<Gateway['CommittedFile']>(
    GIT_FILE,
    { repo, commit: commit ?? '', path: path ?? '' },
    Boolean(commit && path),
  );
  const none = !commit || !path || isProblem(read.error, 'not-committed');
  const data = none ? undefined : read.data;
  return {
    text: none
      ? ''
      : data && !data.binary && !data.too_large
        ? readText(data.text).text
        : undefined,
    error: none ? undefined : read.error,
    binary: data?.binary === true,
    tooLarge: data?.too_large === true,
  };
}
