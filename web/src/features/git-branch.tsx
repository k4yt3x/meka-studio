import * as Popover from '@radix-ui/react-popover';
import { GitBranch, TriangleAlert } from 'lucide-react';
import type { Gateway } from '../api/gateway';
import { useOverlayPadding } from '../components/visual-viewport';

type GitStatus = Gateway['GitStatus'];

/**
 * The branch of a repository, with the commits it is ahead of its upstream and behind; it opens to
 * say all git says of it, as a touch screen shows no tooltip.
 */
export function Branch({ status }: { status: GitStatus }) {
  const collisionPadding = useOverlayPadding();
  const name = status.branch ?? status.commit?.slice(0, 7);
  if (!name) return null;
  const ahead = status.ahead ?? 0;
  const behind = status.behind ?? 0;
  const counts = [ahead ? `↑${ahead}` : '', behind ? `↓${behind}` : ''].filter(Boolean).join(' ');
  const upstream = !status.upstream
    ? 'No upstream'
    : status.ahead == null
      ? `${status.upstream} is gone`
      : ahead && behind
        ? `${ahead} ahead of ${status.upstream}, ${behind} behind`
        : ahead
          ? `${ahead} ahead of ${status.upstream}`
          : behind
            ? `${behind} behind ${status.upstream}`
            : `Up to date with ${status.upstream}`;
  // A commit checked out on its own follows no branch.
  const said = [
    status.branch ? `Branch ${name}` : `Detached at ${name}`,
    status.branch ? upstream : '',
    status.too_many ? 'Too many changes to show' : '',
  ].filter(Boolean);
  return (
    <span className="file-branch-slot">
      <Popover.Root>
        <Popover.Trigger asChild>
          <button type="button" className="file-branch" aria-label={said.join(', ')}>
            <GitBranch size={14} aria-hidden="true" />
            <span className="file-branch-name">{name}</span>
            {counts && <span className="file-branch-counts">{counts}</span>}
            {status.too_many && (
              <TriangleAlert size={13} className="file-branch-warning" aria-hidden="true" />
            )}
          </button>
        </Popover.Trigger>
        <Popover.Portal>
          <Popover.Content
            className="menu-panel branch-details"
            align="end"
            sideOffset={4}
            collisionPadding={collisionPadding}
          >
            <p className="branch-details-name">
              <GitBranch size={14} aria-hidden="true" />
              <span>{status.branch ?? `Detached at ${name}`}</span>
            </p>
            {status.branch && <p>{upstream}</p>}
            {status.branch && status.commit && (
              <p className="muted">At commit {status.commit.slice(0, 7)}</p>
            )}
            {status.too_many && (
              <p className="branch-details-warning">
                <TriangleAlert size={13} aria-hidden="true" />
                <span>Too many changes to show</span>
              </p>
            )}
          </Popover.Content>
        </Popover.Portal>
      </Popover.Root>
    </span>
  );
}
