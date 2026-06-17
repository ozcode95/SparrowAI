import { memo, useCallback } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Renders GitHub-Flavored Markdown inline inside chat bubbles and tool
 * cards. Each element gets a Tailwind override tuned to the surrounding
 * TUI palette so the output sits naturally inside the existing chat
 * surface (no jumbo headings, no margin-collapse halos, no white card
 * backgrounds bleeding through).
 *
 * Links route through the Tauri opener plugin instead of the webview's
 * default navigation — clicking an `https://` URL in a chat reply
 * launches the user's OS default browser, the same way it would from
 * any normal desktop app.
 *
 * Streams safely: react-markdown re-parses on every render, so a
 * half-arrived response (e.g. an unterminated fenced block) still
 * renders the partial AST without crashing.
 */
export const Markdown = memo(function Markdown({
  children,
}: {
  children: string;
}) {
  // Anchor click handler: prevent the webview from navigating *away*
  // from the app (which would blank the entire chat surface) and hand
  // the URL to the OS instead. Falls back to in-app handling only for
  // bare hash anchors so future TOC-style markdown still works inside
  // long replies.
  const onLinkClick = useCallback(
    (e: React.MouseEvent<HTMLAnchorElement>, href: string | undefined) => {
      if (!href) return;
      if (href.startsWith("#")) return; // in-document anchor, let it bubble
      e.preventDefault();
      void openUrl(href).catch((err) => {
        console.error("openUrl failed", href, err);
      });
    },
    [],
  );

  return (
    <div className="markdown-body text-[13px] leading-[1.55] text-tui-fg">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={mdComponents(onLinkClick)}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
});

/**
 * Build the per-element override map. Inlined here (rather than a
 * top-level constant) so we can close over the link handler without
 * recreating it for every leaf node.
 */
function mdComponents(
  onLinkClick: (
    e: React.MouseEvent<HTMLAnchorElement>,
    href: string | undefined,
  ) => void,
): Components {
  return {
    // ─── Block text ────────────────────────────────────────────────
    p: ({ children }) => (
      <p className="my-2 first:mt-0 last:mb-0 whitespace-pre-wrap">
        {children}
      </p>
    ),
    // Headings deliberately stay close in size to body text — chat
    // bubbles are already a visual block, and oversized headers make
    // a 3-bullet reply look like a billboard.
    h1: ({ children }) => (
      <h1 className="mt-3 mb-2 first:mt-0 text-[16px] font-semibold text-tui-fg">
        {children}
      </h1>
    ),
    h2: ({ children }) => (
      <h2 className="mt-3 mb-1.5 first:mt-0 text-[15px] font-semibold text-tui-fg">
        {children}
      </h2>
    ),
    h3: ({ children }) => (
      <h3 className="mt-2.5 mb-1.5 first:mt-0 text-[14px] font-semibold text-tui-fg">
        {children}
      </h3>
    ),
    h4: ({ children }) => (
      <h4 className="mt-2 mb-1 first:mt-0 text-[13px] font-semibold text-tui-fg">
        {children}
      </h4>
    ),
    h5: ({ children }) => (
      <h5 className="mt-2 mb-1 first:mt-0 text-[13px] font-semibold uppercase tracking-wide text-tui-fg-dim">
        {children}
      </h5>
    ),
    h6: ({ children }) => (
      <h6 className="mt-2 mb-1 first:mt-0 text-[12px] font-semibold uppercase tracking-wide text-tui-fg-muted">
        {children}
      </h6>
    ),

    // ─── Inline ───────────────────────────────────────────────────
    strong: ({ children }) => (
      <strong className="font-semibold text-tui-fg">{children}</strong>
    ),
    em: ({ children }) => <em className="italic">{children}</em>,
    del: ({ children }) => (
      <del className="text-tui-fg-muted line-through">{children}</del>
    ),
    a: ({ href, children, title }) => (
      <a
        href={href}
        title={title}
        onClick={(e) => onLinkClick(e, href)}
        className="text-tui-accent underline decoration-tui-accent/40 underline-offset-2 hover:decoration-tui-accent"
      >
        {children}
      </a>
    ),

    // ─── Lists ────────────────────────────────────────────────────
    ul: ({ children }) => (
      <ul className="my-2 ml-5 list-disc space-y-1 marker:text-tui-fg-muted">
        {children}
      </ul>
    ),
    ol: ({ children }) => (
      <ol className="my-2 ml-5 list-decimal space-y-1 marker:text-tui-fg-muted">
        {children}
      </ol>
    ),
    li: ({ children }) => <li className="whitespace-pre-wrap">{children}</li>,

    // ─── Quotes / rules ───────────────────────────────────────────
    blockquote: ({ children }) => (
      <blockquote className="my-2 border-l-2 border-tui-accent/60 pl-3 text-tui-fg-dim italic">
        {children}
      </blockquote>
    ),
    hr: () => <hr className="my-3 border-t border-tui-border" />,

    // ─── Code ─────────────────────────────────────────────────────
    // react-markdown 9+ no longer passes an explicit `inline` flag;
    // the convention is to detect block-vs-inline by whether the
    // parent is a <pre>. We do the simpler heuristic: if the rendered
    // children contain a newline it's a block, otherwise inline.
    code: ({ children, className }) => {
      const text = String(children ?? "");
      const isBlock = text.includes("\n");
      if (isBlock) {
        // The outer <pre> override below renders the chrome; here we
        // just preserve the language class so future syntax-highlighter
        // wiring has something to grab onto.
        return (
          <code className={className ?? ""} style={{ display: "block" }}>
            {children}
          </code>
        );
      }
      return (
        <code className="rounded bg-[var(--fluent-bg-subtle)] px-1 py-0.5 font-mono text-[12px] text-tui-fg">
          {children}
        </code>
      );
    },
    pre: ({ children }) => (
      <pre className="my-2 overflow-x-auto rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] px-3 py-2 font-mono text-[12px] leading-[1.5] text-tui-fg">
        {children}
      </pre>
    ),

    // ─── Tables (GFM) ─────────────────────────────────────────────
    table: ({ children }) => (
      <div className="my-2 overflow-x-auto">
        <table className="min-w-full border-collapse border border-tui-border text-[12px]">
          {children}
        </table>
      </div>
    ),
    thead: ({ children }) => (
      <thead className="bg-[var(--fluent-bg-subtle)] text-tui-fg">
        {children}
      </thead>
    ),
    tbody: ({ children }) => <tbody>{children}</tbody>,
    tr: ({ children }) => (
      <tr className="border-b border-tui-border last:border-b-0">{children}</tr>
    ),
    th: ({ children, style }) => (
      <th
        style={style}
        className="border border-tui-border px-2 py-1 text-left font-semibold"
      >
        {children}
      </th>
    ),
    td: ({ children, style }) => (
      <td
        style={style}
        className="border border-tui-border px-2 py-1 align-top"
      >
        {children}
      </td>
    ),

    // ─── Images ───────────────────────────────────────────────────
    img: ({ src, alt, title }) => (
      <img
        src={src}
        alt={alt ?? ""}
        title={title}
        className="my-2 max-w-full rounded border border-tui-border"
        loading="lazy"
      />
    ),
  };
}
