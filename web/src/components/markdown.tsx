import {
  useState,
  isValidElement,
  memo,
  type ComponentProps,
  type ReactElement,
  type ReactNode,
} from 'react';
import ReactMarkdown, { defaultUrlTransform } from 'react-markdown';
import remarkCjkFriendly from 'remark-cjk-friendly/parseOnly';
import remarkGfm from 'remark-gfm';
import rehypeKatex from 'rehype-katex';
import { CircleAlert, Info, Lightbulb, OctagonAlert } from 'lucide-react';
import { fileLink, type FileLink } from '../files/paths';
import { CopyButton, WrapButton } from './common';
import { Diagram } from './diagram';
import { MarkdownTable } from './markdown-table';
import { Checkbox } from './ui/checkbox';
import { scrollRegion } from './scrolling';
import { useHighlight } from './use-highlight';
import {
  katexOptions,
  protectTableMath,
  remarkDisplayMath,
  remarkMath,
  rehypeMathAccessibility,
  rehypeMathSource,
} from './markdown-math';
import { remarkAdmonitions, rehypeTaskLabels } from './markdown-plugins';
import { useWorkspace } from './workspace';
import 'katex/dist/katex.min.css';

export function MarkdownPreview({ text }: { text: string }) {
  // A collapsed row needs only a bounded opening excerpt, without links, images, or heavy renderers.
  const source = text.trim();
  const preview = source.slice(0, 1024).replace(/\s+/g, ' ');
  return (
    <ReactMarkdown
      remarkPlugins={[remarkCjkFriendly]}
      allowedElements={['strong', 'em', 'del', 'code']}
      unwrapDisallowed
      skipHtml
    >
      {preview + (source.length > 1024 ? '…' : '')}
    </ReactMarkdown>
  );
}

export function CodeBlock({ text, language }: { text: string; language: string | undefined }) {
  const tokens = useHighlight(text, language);
  const [wrap, setWrap] = useState(false);
  return (
    <div className="code-block" data-wrap={wrap || undefined}>
      <div className="code-toolbar">
        <span>{language ?? 'text'}</span>
        <div className="code-actions">
          <WrapButton wrap={wrap} onWrapChange={setWrap} />
          <CopyButton text={text} />
        </div>
      </div>
      <pre>
        <code
          tabIndex={0}
          role="group"
          aria-label={language ? `${language} code` : 'Code'}
          onKeyDown={scrollRegion}
        >
          {tokens
            ? tokens.map((line, index) => (
                <span className="code-line" key={index}>
                  {line.map((token, i) => (
                    <span key={i} style={token.htmlStyle}>
                      {token.content}
                    </span>
                  ))}
                  {index < tokens.length - 1 ? '\n' : ''}
                </span>
              ))
            : text}
        </code>
      </pre>
    </div>
  );
}
function FencedBlock({ children }: ComponentProps<'pre'>) {
  if (!isValidElement(children)) return <pre>{children}</pre>;
  const code = children as ReactElement<ComponentProps<'code'>>;
  const text = String(code.props.children ?? '').replace(/\n$/, '');
  const language = /language-([^\s]+)/.exec(code.props.className ?? '')?.[1];
  return language?.toLowerCase() === 'mermaid' ? (
    <Diagram source={text} />
  ) : (
    <CodeBlock text={text} language={language} />
  );
}
/** A link to a file, which opens it in the editor, or to a folder, which opens it in Files. */
function FileAnchor({
  link,
  id,
  children,
}: {
  link: FileLink;
  id: string | undefined;
  children: ReactNode;
}) {
  const workspace = useWorkspace();
  const open = () =>
    link.folder
      ? workspace?.openFiles(link.path)
      : workspace?.openFile(link.path, { lines: link.lines });
  const { lines } = link;
  return (
    <a
      role="link"
      tabIndex={0}
      className="file-link"
      id={id}
      title={lines ? `${link.path}:${lines.line}${lines.end ? `-${lines.end}` : ''}` : link.path}
      onClick={open}
      onKeyDown={(event) => {
        if (event.key === 'Enter') open();
      }}
    >
      {children}
    </a>
  );
}
const alertTitles: Record<string, string> = {
  note: 'Note',
  tip: 'Tip',
  important: 'Important',
  warning: 'Warning',
  caution: 'Caution',
};
export const Markdown = memo(function Markdown({
  text,
  base,
}: {
  text: string;
  /** The folder a relative link to a file starts from; without it, links keep their addresses. */
  base?: string | undefined;
}) {
  // Links to files open in the session's editor, which only a session's workspace has.
  const folder = useWorkspace() ? base : undefined;
  return (
    <div
      className="markdown"
      onKeyDown={(event) => {
        if (event.target instanceof HTMLElement && event.target.classList.contains('katex-display'))
          scrollRegion(event, event.target);
      }}
    >
      <ReactMarkdown
        remarkPlugins={[
          remarkGfm,
          remarkCjkFriendly,
          remarkMath,
          remarkDisplayMath,
          remarkAdmonitions,
        ]}
        rehypePlugins={[
          rehypeTaskLabels,
          rehypeMathSource,
          [rehypeKatex, katexOptions],
          rehypeMathAccessibility,
        ]}
        skipHtml
        urlTransform={(url, key) =>
          key === 'href' && folder !== undefined && fileLink(url, folder)
            ? url
            : defaultUrlTransform(url)
        }
        components={{
          pre: FencedBlock,
          table: MarkdownTable,
          input: ({ type, checked, 'aria-label': label }) =>
            type === 'checkbox' ? (
              <Checkbox checked={Boolean(checked)} disabled aria-label={label} />
            ) : null,
          blockquote: ({ children, node }) => {
            const kind = String(node?.properties['data-alert'] ?? node?.properties.dataAlert ?? '');
            if (!Object.hasOwn(alertTitles, kind)) return <blockquote>{children}</blockquote>;
            const Icon =
              kind === 'tip'
                ? Lightbulb
                : kind === 'caution'
                  ? OctagonAlert
                  : kind === 'warning'
                    ? CircleAlert
                    : Info;
            return (
              <aside
                className={`markdown-alert markdown-alert-${kind}`}
                role="note"
                aria-label={alertTitles[kind]}
              >
                <div className="alert-heading">
                  <Icon size={16} />
                  {alertTitles[kind]}
                </div>
                {children}
              </aside>
            );
          },
          a: ({ children, href, title, id, node }) => {
            const link = href && folder !== undefined ? fileLink(href, folder) : undefined;
            if (link)
              return (
                <FileAnchor link={link} id={id}>
                  {children}
                </FileAnchor>
              );
            const fragment = href?.startsWith('#');
            return (
              <a
                href={href}
                title={title}
                id={id}
                target={fragment ? undefined : '_blank'}
                rel={fragment ? undefined : 'noreferrer noopener'}
                aria-label={node?.properties.dataFootnoteBackref ? 'Back to reference' : undefined}
                onClick={
                  fragment
                    ? (event) => {
                        event.preventDefault();
                        const target = event.currentTarget
                          .closest('.markdown')
                          ?.querySelector<HTMLElement>(
                            `[id="${CSS.escape(href?.slice(1) ?? '')}"]`,
                          );
                        target?.scrollIntoView({ block: 'nearest' });
                        target?.focus({ preventScroll: true });
                      }
                    : undefined
                }
              >
                {children}
              </a>
            );
          },
          img: ({ src, alt }) => (
            <a href={src} target="_blank" rel="noreferrer noopener">
              {alt || 'External image'} (open image)
            </a>
          ),
        }}
      >
        {protectTableMath(text)}
      </ReactMarkdown>
    </div>
  );
});
