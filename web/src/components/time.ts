/**
 * How long ago `value`, a time, was, as a person says it: "3 minutes ago", "yesterday", and past
 * a week, the date; none for a time that cannot be read.
 */
export function ago(value: string): string | undefined {
  const minutes = (Date.now() - Date.parse(value)) / 60_000;
  if (!Number.isFinite(minutes)) return undefined;
  const relative = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
  if (minutes < 60) return relative.format(-Math.round(Math.max(minutes, 0)), 'minute');
  if (minutes < 24 * 60) return relative.format(-Math.round(minutes / 60), 'hour');
  if (minutes < 7 * 24 * 60) return relative.format(-Math.round(minutes / (24 * 60)), 'day');
  return new Date(value).toLocaleDateString();
}
