import { readdirSync, readFileSync } from 'node:fs';
import { expect, it } from 'vitest';

const GUARD = ":where(:not([data-surface^='monaco'] *))";

/** The class names Monaco's stylesheets use, which its widgets carry. */
function monacoClasses() {
  const root = new URL('../../node_modules/monaco-editor/esm/vs/', import.meta.url);
  const names = new Set<string>();
  for (const path of readdirSync(root, { recursive: true, encoding: 'utf8' })) {
    if (!path.endsWith('.css')) continue;
    const css = readFileSync(new URL(path, root), 'utf8');
    for (const [, name = ''] of css.matchAll(/\.(-?[_a-zA-Z][\w-]*)/g)) names.add(name);
  }
  return names;
}

/** Each selector of the page's style rules, with the at-rules around it. */
function selectors() {
  const css = readFileSync(new URL('../styles.css', import.meta.url), 'utf8').replace(
    /\/\*[\s\S]*?\*\//g,
    '',
  );
  const found: { selector: string; context: string[] }[] = [];
  const open: string[] = [];
  for (const [token, head = ''] of css.matchAll(/([^{}]*)\{|\}/g)) {
    if (token === '}') {
      open.pop();
      continue;
    }
    const prelude = head.trim();
    const inside = open.slice();
    open.push(prelude);
    if (prelude.startsWith('@') || inside.some((rule) => rule.startsWith('@keyframes'))) continue;
    let depth = 0;
    let start = 0;
    for (let index = 0; index <= prelude.length; index++) {
      const char = prelude[index];
      if (char === '(') depth++;
      else if (char === ')') depth--;
      else if (index === prelude.length || (char === ',' && depth === 0)) {
        const selector = prelude.slice(start, index).replace(/\s+/g, ' ').trim();
        found.push({
          selector: selector.replace(/\( /g, '(').replace(/ \)/g, ')'),
          context: inside,
        });
        start = index + 1;
      }
    }
  }
  return found;
}

it('keeps the page styles that could match Monaco widgets off its surface', () => {
  const shared = monacoClasses();
  const reaching = selectors().filter(({ selector, context }) => {
    // Less motion, as the system asks or Animation is off, stills Monaco's widgets as well.
    if (context.some((rule) => rule.includes('prefers-reduced-motion'))) return false;
    if (selector.startsWith(":root[data-animation='off']")) return false;
    if (selector.startsWith("[data-surface^='monaco']")) return false;
    const subject = selector.split(/\s*[\s>+~]\s*(?![^(]*\))/).at(-1)!;
    if (/^(:root|html|body)\b/.test(subject)) return false;
    const classes = [...selector.replace(GUARD, '').matchAll(/\.(-?[_a-zA-Z][\w-]*)/g)];
    return classes.every(([, name = '']) => shared.has(name));
  });
  expect(reaching.length).toBeGreaterThan(0);
  expect(reaching.filter(({ selector }) => !selector.includes(GUARD))).toEqual([]);
});
