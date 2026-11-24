/**
 * The arguments `text` writes: separated by white space, where single or double quotes keep a part
 * together. Backslashes are kept as written, as Windows paths need, and a quote left open runs to
 * the end.
 */
export function splitArgs(text: string): string[] {
  const args: string[] = [];
  let current: string | undefined;
  let quote: string | undefined;
  for (const char of text) {
    if (quote) {
      if (char === quote) quote = undefined;
      else current = (current ?? '') + char;
    } else if (char === '"' || char === "'") {
      quote = char;
      current ??= '';
    } else if (/\s/.test(char)) {
      if (current !== undefined) args.push(current);
      current = undefined;
    } else {
      current = (current ?? '') + char;
    }
  }
  if (current !== undefined) args.push(current);
  return args;
}

/** `args` written as {@link splitArgs} reads them. */
export function joinArgs(args: readonly string[]): string {
  return args
    .map((arg) => {
      if (arg && !/[\s"']/.test(arg)) return arg;
      if (!arg.includes('"')) return `"${arg}"`;
      if (!arg.includes("'")) return `'${arg}'`;
      // Each double quote closes the quoted part, goes in single quotes, and opens it again.
      return `"${arg.replaceAll('"', `"'"'"`)}"`;
    })
    .join(' ');
}
