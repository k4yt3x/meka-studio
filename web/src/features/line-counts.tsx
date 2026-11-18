/** The lines a change adds and removes, in their colours, as git counts them. */
export function LineCounts({ added, removed }: { added: number; removed: number }) {
  return (
    <span
      className="line-counts"
      aria-label={`${added.toLocaleString()} ${added === 1 ? 'line' : 'lines'} added, ${removed.toLocaleString()} removed`}
    >
      <span className="line-counts-added">+{added.toLocaleString()}</span>
      <span className="line-counts-removed">−{removed.toLocaleString()}</span>
    </span>
  );
}
