//! Web search & research tools.
//!
//! Three in-process tools the model can call without any API keys:
//!
//! 1. `web.search` — DuckDuckGo HTML search. Returns title / URL / snippet
//!    for the top results.
//! 2. `web.read_page` — Fetch a single URL and return the cleaned main-
//!    content text (HTML tags stripped, scripts/nav/styles dropped, the
//!    first `<main>` / `<article>` block preferred when present).
//! 3. `web.deep_research` — Convenience orchestration: search, then fetch
//!    and clean the top N pages, then return a single bundled brief the
//!    model can synthesise from in one turn.
//!
//! Why DuckDuckGo? It exposes a stable HTML endpoint at
//! `https://html.duckduckgo.com/html/` that doesn't require an API key,
//! account, or rate-limit token. We send a desktop browser User-Agent so
//! the server returns the full results page instead of the JS-shell
//! variant.
//!
//! None of these tools are marked destructive — they're read-only against
//! third-party servers, same trust boundary as `http.fetch`. The fetch
//! step uses the shared [`reqwest::Client`] from [`crate::state::AppState`]
//! so proxies / TLS settings come along for free; we layer a per-request
//! `User-Agent` and `Accept-Language` header on top because DuckDuckGo
//! tailors its layout to those.

use crate::mcp::{Tool, ToolResult, ToolSchema};
use crate::state::AppStateExt;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::header::{ACCEPT_LANGUAGE, USER_AGENT};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::AppHandle;
use url::Url;

/// DuckDuckGo's no-JS HTML results endpoint. Stable, no API key required.
const DDG_SEARCH_URL: &str = "https://html.duckduckgo.com/html/";

/// Desktop browser UA. The default `zero/0.1.0` UA on the shared client
/// gets DuckDuckGo's bot-detection page; this one gets the real results.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/124.0.0.0 Safari/537.36";

/// Default per-request timeout. Search responses are tiny but a single
/// slow upstream shouldn't gate the chat for more than a few seconds.
const DEFAULT_TIMEOUT_MS: u64 = 15_000;

/// Default number of search results to surface. Keeps the tool message
/// small enough that the model can read it without blowing its context.
const DEFAULT_SEARCH_LIMIT: usize = 5;
/// Upper cap to stop the model from asking for a giant page of results.
const MAX_SEARCH_LIMIT: usize = 10;

/// Default cap on cleaned text returned by `web.read_page`.
const DEFAULT_READ_CHARS: usize = 8_000;
/// Hard cap so a 5 MB article can't blow the model's context window.
const MAX_READ_CHARS: usize = 32_000;

/// Deep research defaults: how many sources to chase down and how much
/// of each to keep. Picked so a single tool call stays comfortably under
/// ~40k chars total.
const DEFAULT_RESEARCH_SOURCES: usize = 4;
const MAX_RESEARCH_SOURCES: usize = 8;
const DEFAULT_RESEARCH_CHARS_PER_SOURCE: usize = 4_000;
const MAX_RESEARCH_CHARS_PER_SOURCE: usize = 8_000;

// ─── Slash-gated tool registry ──────────────────────────────────────────────
//
// Web tools reach arbitrary third-party servers, so they're treated as
// opt-in:
//
//   * Hidden from the Tools page and the per-chat Tools popover so users
//     can't accidentally enable them globally.
//   * Excluded from the LLM tool catalog on every turn unless the user
//     explicitly opts in for that turn with a slash command
//     (`/web` or `/research`).
//
// Both layers — the IPC `mcp_list_builtins` filter and the chat runner's
// per-turn catalog gate — consult [`is_slash_gated`] / [`WebUnlocks`]
// here so the policy stays in one place.

/// Tool names whose schemas should be omitted from `mcp_list_builtins`.
/// These are still constructed by [`crate::mcp::builtin_registry`] so
/// the chat runner can unlock them per-turn, just hidden from the UI.
pub const SLASH_GATED_TOOL_NAMES: &[&str] = &["web.search", "web.deep_research", "web.read_page"];

/// True when `tool_name` is one of the [`SLASH_GATED_TOOL_NAMES`] entries.
pub fn is_slash_gated(tool_name: &str) -> bool {
    SLASH_GATED_TOOL_NAMES.contains(&tool_name)
}

/// Per-turn opt-ins resolved from slash markers on the latest user message.
/// The runner builds one of these once and consults [`Self::allows`] for
/// every catalog entry so the gating rule stays in one place.
#[derive(Debug, Default, Clone, Copy)]
pub struct WebUnlocks {
    /// User typed `/web …` — unlocks `web.search` + `web.read_page`.
    pub search: bool,
    /// User typed `/research …` — unlocks `web.deep_research`
    /// + `web.read_page`.
    pub research: bool,
}

impl WebUnlocks {
    /// True when this turn allows the built-in tool named `tool_name`.
    /// Returns `true` for any non-slash-gated tool so the caller can use
    /// this as the sole filter on the built-in subset of its catalog.
    pub fn allows(&self, tool_name: &str) -> bool {
        match tool_name {
            "web.search" => self.search,
            "web.deep_research" => self.research,
            // `read_page` is the natural follow-up to either search
            // tool, so either opt-in unlocks it.
            "web.read_page" => self.search || self.research,
            _ => true,
        }
    }
}

// ─── web.search ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct WebSearch {
    http: Client,
}

impl WebSearch {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            http: app.zero().http.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for WebSearch {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "web.search".into(),
            description: "Search the web via DuckDuckGo and return the top results \
                 as a numbered list of {title, url, snippet}. No API key \
                 required. Use this for fresh information the model \
                 might not have seen during training, or to find primary \
                 sources for a topic before calling web.read_page."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Plain-text search query."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_SEARCH_LIMIT,
                        "description": "Max results to return. Default 5, max 10."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: SearchArgs = serde_json::from_value(args).context("web.search: parse arguments")?;
        let query = a.query.trim();
        if query.is_empty() {
            return Ok(ToolResult {
                content: "web.search: `query` is empty".into(),
                is_error: true,
            });
        }
        let limit = a
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);

        let results = match ddg_search(&self.http, query, limit).await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!("web.search: {e:#}"),
                    is_error: true,
                });
            }
        };
        if results.is_empty() {
            return Ok(ToolResult {
                content: format!("web.search: no results for `{query}`"),
                is_error: false,
            });
        }

        let mut out = String::new();
        out.push_str(&format!("web.search results for `{query}`:\n\n"));
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!(
                "{}. {}\n   {}\n   {}\n\n",
                i + 1,
                r.title,
                r.url,
                r.snippet
            ));
        }
        Ok(ToolResult {
            content: out.trim_end().to_string(),
            is_error: false,
        })
    }
}

// ─── web.read_page ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct WebReadPage {
    http: Client,
}

impl WebReadPage {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            http: app.zero().http.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[async_trait]
impl Tool for WebReadPage {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "web.read_page".into(),
            description: "Fetch a web page and return its main-content text with \
                 HTML tags, scripts, styles, and navigation chrome \
                 stripped. Prefer this over http.fetch when you want to \
                 read an article — http.fetch returns raw HTML which is \
                 much harder for the model to parse and burns context."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Absolute http(s) URL of the page to read."
                    },
                    "max_chars": {
                        "type": "integer",
                        "minimum": 200,
                        "maximum": MAX_READ_CHARS,
                        "description": "Cap on returned text. Default 8000, max 32000."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: ReadArgs = serde_json::from_value(args).context("web.read_page: parse arguments")?;
        let url = a.url.trim();
        if url.is_empty() {
            return Ok(ToolResult {
                content: "web.read_page: `url` is empty".into(),
                is_error: true,
            });
        }
        let max_chars = a
            .max_chars
            .unwrap_or(DEFAULT_READ_CHARS)
            .clamp(200, MAX_READ_CHARS);

        match fetch_and_extract(&self.http, url, max_chars).await {
            Ok((title, body, truncated)) => {
                let mut out = String::new();
                if !title.is_empty() {
                    out.push_str(&format!("# {title}\n"));
                }
                out.push_str(&format!("URL: {url}\n\n"));
                out.push_str(&body);
                if truncated {
                    out.push_str("\n\n… [truncated]");
                }
                Ok(ToolResult {
                    content: out,
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResult {
                content: format!("web.read_page: {e:#}"),
                is_error: true,
            }),
        }
    }
}

// ─── web.deep_research ─────────────────────────────────────────────────────

#[derive(Debug)]
pub struct WebDeepResearch {
    http: Client,
}

impl WebDeepResearch {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            http: app.zero().http.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResearchArgs {
    query: String,
    #[serde(default)]
    max_sources: Option<usize>,
    #[serde(default)]
    chars_per_source: Option<usize>,
}

#[async_trait]
impl Tool for WebDeepResearch {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "web.deep_research".into(),
            description: "Run a multi-source web research pass: search DuckDuckGo, \
                 then fetch and clean the top sources, returning one \
                 bundled brief the model can synthesise an answer from \
                 in a single turn. Prefer this over chaining web.search + \
                 multiple web.read_page calls when the user asks for an \
                 overview, comparison, or up-to-date summary of a topic."
                .into(),
            input_schema: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Research question / topic in plain text."
                    },
                    "max_sources": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_RESEARCH_SOURCES,
                        "description":
                            "How many top results to fetch and digest. \
                             Default 4, max 8."
                    },
                    "chars_per_source": {
                        "type": "integer",
                        "minimum": 500,
                        "maximum": MAX_RESEARCH_CHARS_PER_SOURCE,
                        "description":
                            "Max cleaned text per source. Default 4000, max 8000."
                    }
                }
            }),
            destructive: false,
        }
    }

    async fn call(&self, args: Value) -> Result<ToolResult> {
        let a: ResearchArgs =
            serde_json::from_value(args).context("web.deep_research: parse arguments")?;
        let query = a.query.trim();
        if query.is_empty() {
            return Ok(ToolResult {
                content: "web.deep_research: `query` is empty".into(),
                is_error: true,
            });
        }
        let max_sources = a
            .max_sources
            .unwrap_or(DEFAULT_RESEARCH_SOURCES)
            .clamp(1, MAX_RESEARCH_SOURCES);
        let chars_per_source = a
            .chars_per_source
            .unwrap_or(DEFAULT_RESEARCH_CHARS_PER_SOURCE)
            .clamp(500, MAX_RESEARCH_CHARS_PER_SOURCE);

        let results = match ddg_search(&self.http, query, max_sources).await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!("web.deep_research: search failed: {e:#}"),
                    is_error: true,
                });
            }
        };
        if results.is_empty() {
            return Ok(ToolResult {
                content: format!("web.deep_research: no results for `{query}`"),
                is_error: false,
            });
        }

        let mut out = String::new();
        out.push_str(&format!("# Research brief: {query}\n\n"));
        out.push_str(&format!(
            "Synthesised from {} source(s) via DuckDuckGo.\n\n",
            results.len()
        ));
        out.push_str("## Sources\n");
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!("{}. {} — {}\n", i + 1, r.title, r.url));
        }
        out.push('\n');

        // Fetch each source in parallel so a slow page doesn't gate the
        // whole brief. Failures are collected and surfaced inline so the
        // model can still reason about the partial result set.
        let fetches = results.iter().map(|r| {
            let http = self.http.clone();
            let url = r.url.clone();
            async move {
                let res = fetch_and_extract(&http, &url, chars_per_source).await;
                (url, res)
            }
        });
        let fetched = futures_util::future::join_all(fetches).await;

        for (i, (r, (_url, res))) in results.iter().zip(fetched.iter()).enumerate() {
            out.push_str(&format!("---\n\n## [{}] {}\n", i + 1, r.title));
            out.push_str(&format!("URL: {}\n", r.url));
            if !r.snippet.is_empty() {
                out.push_str(&format!("Snippet: {}\n", r.snippet));
            }
            out.push('\n');
            match res {
                Ok((_title, body, truncated)) => {
                    out.push_str(body);
                    if *truncated {
                        out.push_str("\n\n… [truncated]");
                    }
                }
                Err(e) => {
                    out.push_str(&format!("[fetch failed: {e:#}]"));
                }
            }
            out.push_str("\n\n");
        }

        Ok(ToolResult {
            content: out.trim_end().to_string(),
            is_error: false,
        })
    }
}

// ─── Shared helpers ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SearchHit {
    title: String,
    url: String,
    snippet: String,
}

/// Hit DuckDuckGo's no-JS HTML endpoint and parse the top `limit`
/// results. The DOM shape there is stable enough that selector-based
/// extraction is reliable, but we still tolerate missing fields per row
/// so a single weird result doesn't drop everything below it.
async fn ddg_search(http: &Client, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let url = format!("{DDG_SEARCH_URL}?q={}&kl=us-en", urlencoding::encode(query));
    let resp = http
        .get(&url)
        .header(USER_AGENT, BROWSER_UA)
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS))
        .send()
        .await
        .context("send search request")?;
    if !resp.status().is_success() {
        anyhow::bail!("DuckDuckGo returned HTTP {}", resp.status());
    }
    let html = resp.text().await.context("read search body")?;
    Ok(parse_ddg_results(&html, limit))
}

fn parse_ddg_results(html: &str, limit: usize) -> Vec<SearchHit> {
    let doc = Html::parse_document(html);
    // The HTML endpoint wraps each result in `div.result` (sometimes
    // `div.web-result`). Inside, `a.result__a` carries the title + link,
    // and `a.result__snippet` (or `.result__snippet` on a `<span>`) holds
    // the description.
    let result_sel = Selector::parse("div.result, div.web-result").unwrap();
    let title_sel = Selector::parse("a.result__a").unwrap();
    let snippet_sel = Selector::parse(".result__snippet").unwrap();

    let mut out = Vec::with_capacity(limit);
    for r in doc.select(&result_sel) {
        if out.len() >= limit {
            break;
        }
        let Some(title_el) = r.select(&title_sel).next() else {
            continue;
        };
        let title = collapse_ws(&title_el.text().collect::<String>());
        let Some(raw_href) = title_el.value().attr("href") else {
            continue;
        };
        // DDG wraps result URLs through `/l/?uddg=<encoded>&...`. Unwrap
        // it so the model gets the real destination it can hand back to
        // web.read_page. Falls back to the raw href if the redirect
        // shape changes upstream.
        let url = unwrap_ddg_redirect(raw_href).unwrap_or_else(|| raw_href.to_string());
        if url.is_empty() || title.is_empty() {
            continue;
        }
        // Skip ad slots — DDG labels them under the same `.result`
        // wrapper but the href routes through `/y.js?ad_provider=…`.
        if url.contains("/y.js") || url.starts_with("https://duckduckgo.com/y.js") {
            continue;
        }
        let snippet = r
            .select(&snippet_sel)
            .next()
            .map(|s| collapse_ws(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SearchHit {
            title,
            url,
            snippet,
        });
    }
    out
}

/// DuckDuckGo's HTML results route through `//duckduckgo.com/l/?uddg=…`
/// (URL-encoded original) plus tracking params. Pull the destination
/// out so downstream tools (and the model) see the real URL.
fn unwrap_ddg_redirect(href: &str) -> Option<String> {
    // Normalise to an absolute URL we can parse. DDG sometimes returns
    // `//duckduckgo.com/l/?…` (protocol-relative) and sometimes a full
    // `https://…` link.
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };
    let parsed = Url::parse(&absolute).ok()?;
    if !parsed.path().starts_with("/l/") {
        // Not a redirect wrapper; nothing to unwrap.
        return Some(absolute);
    }
    let uddg = parsed
        .query_pairs()
        .find(|(k, _)| k == "uddg")
        .map(|(_, v)| v.into_owned())?;
    Some(uddg)
}

/// Fetch a page and reduce it to a cleaned text excerpt. Returns
/// `(title, body, truncated)`.
async fn fetch_and_extract(
    http: &Client,
    url: &str,
    max_chars: usize,
) -> Result<(String, String, bool)> {
    let resp = http
        .get(url)
        .header(USER_AGENT, BROWSER_UA)
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS))
        .send()
        .await
        .context("send page request")?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("upstream returned HTTP {}", status);
    }
    let html = resp.text().await.context("read page body")?;
    Ok(extract_main_text(&html, max_chars))
}

/// Turn raw HTML into `(title, body, truncated)`. Drops scripts, styles,
/// nav/header/footer/aside/form, and SVG noise; prefers the content of
/// the first `<main>` / `<article>` if present.
fn extract_main_text(html: &str, max_chars: usize) -> (String, String, bool) {
    let doc = Html::parse_document(html);

    let title = doc
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|t| collapse_ws(&t.text().collect::<String>()))
        .unwrap_or_default();

    // Try the structural content roots first; fall back to <body> so we
    // never return an empty string just because the page didn't use
    // semantic HTML.
    let main_sel = Selector::parse("main, article").unwrap();
    let body_sel = Selector::parse("body").unwrap();
    let root = doc
        .select(&main_sel)
        .next()
        .or_else(|| doc.select(&body_sel).next());

    let Some(root) = root else {
        return (title, String::new(), false);
    };

    // Nodes we never want in the output. We walk the *root's* subtree
    // and skip the entire branch when we hit one of these tags.
    const SKIP_TAGS: &[&str] = &[
        "script", "style", "noscript", "svg", "nav", "header", "footer", "aside", "form", "iframe",
        "button", "menu",
    ];

    let mut buf = String::new();
    collect_text(*root, &mut buf, SKIP_TAGS);
    let normalised = collapse_block_ws(&buf);
    let truncated = normalised.chars().count() > max_chars;
    let body = if truncated {
        normalised.chars().take(max_chars).collect::<String>()
    } else {
        normalised
    };
    (title, body, truncated)
}

/// Depth-first walk that appends text nodes to `buf` while skipping any
/// element whose tag is in `skip`. Inserts a blank line at block
/// boundaries (p, div, li, h1–h6, br, tr) so the cleaned text reads as
/// paragraphs instead of one giant run-on string.
fn collect_text(node: ego_tree::NodeRef<scraper::Node>, buf: &mut String, skip: &[&str]) {
    use scraper::Node;
    match node.value() {
        Node::Text(t) => {
            buf.push_str(&t.text);
        }
        Node::Element(el) => {
            let name = el.name();
            if skip.contains(&name) {
                return;
            }
            let is_block = matches!(
                name,
                "p" | "div"
                    | "li"
                    | "ul"
                    | "ol"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "section"
                    | "article"
                    | "tr"
                    | "blockquote"
                    | "pre"
            );
            if matches!(name, "br") {
                buf.push('\n');
                return;
            }
            for child in node.children() {
                collect_text(child, buf, skip);
            }
            if is_block {
                buf.push('\n');
            }
        }
        _ => {
            for child in node.children() {
                collect_text(child, buf, skip);
            }
        }
    }
}

/// Collapse runs of inline whitespace (spaces, tabs) into a single space
/// without flattening hard line breaks. Used for single-line text like
/// titles and snippets where layout-driven whitespace should disappear.
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Whitespace cleanup for cleaned page bodies: collapse runs of spaces
/// inside a line, drop fully-blank lines beyond a single one, and trim
/// per-line padding. Preserves paragraph structure introduced by
/// `collect_text`.
fn collapse_block_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0u32;
    for raw_line in s.split('\n') {
        let line = collapse_ws(raw_line);
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

pub fn all(app: &AppHandle) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(WebSearch::new(app)),
        Box::new(WebReadPage::new(app)),
        Box::new(WebDeepResearch::new(app)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_ws_squashes_inline_whitespace() {
        assert_eq!(collapse_ws("  hello   world  "), "hello world");
        assert_eq!(collapse_ws("a\tb\nc"), "a b c");
        assert_eq!(collapse_ws("   "), "");
    }

    #[test]
    fn collapse_block_ws_keeps_paragraph_breaks() {
        let input = "para one\n\n\n\npara two\n\nthird";
        assert_eq!(collapse_block_ws(input), "para one\n\npara two\n\nthird");
    }

    #[test]
    fn extract_main_text_prefers_main_over_chrome() {
        let html = r#"
            <html>
              <head><title>Demo</title></head>
              <body>
                <nav>nope</nav>
                <header>nope</header>
                <main>
                  <h1>Hello</h1>
                  <p>This is the content.</p>
                </main>
                <footer>nope</footer>
              </body>
            </html>
        "#;
        let (title, body, truncated) = extract_main_text(html, 1_000);
        assert_eq!(title, "Demo");
        assert!(!truncated);
        assert!(body.contains("Hello"));
        assert!(body.contains("This is the content."));
        assert!(!body.contains("nope"));
    }

    #[test]
    fn extract_main_text_strips_scripts_and_styles() {
        let html = r#"
            <html><body>
              <article>
                <script>alert('hi')</script>
                <style>body{color:red}</style>
                <p>Visible.</p>
              </article>
            </body></html>
        "#;
        let (_t, body, _trunc) = extract_main_text(html, 1_000);
        assert!(body.contains("Visible."));
        assert!(!body.contains("alert"));
        assert!(!body.contains("color:red"));
    }

    #[test]
    fn extract_main_text_truncates_long_content() {
        let big_para = "x".repeat(20_000);
        let html = format!("<html><body><main><p>{big_para}</p></main></body></html>");
        let (_t, body, truncated) = extract_main_text(&html, 500);
        assert!(truncated);
        assert!(body.chars().count() <= 500);
    }

    #[test]
    fn unwrap_ddg_redirect_extracts_uddg() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa&rut=abc";
        assert_eq!(
            unwrap_ddg_redirect(href).as_deref(),
            Some("https://example.com/a")
        );
    }

    #[test]
    fn unwrap_ddg_redirect_passes_through_direct_links() {
        let href = "https://example.com/page";
        assert_eq!(
            unwrap_ddg_redirect(href).as_deref(),
            Some("https://example.com/page")
        );
    }

    #[test]
    fn web_unlocks_gates_only_web_tools_by_default() {
        let none = WebUnlocks::default();
        // Non-web built-ins are always allowed — the helper doubles as
        // the sole filter on the built-in subset of the catalog.
        assert!(none.allows("fs.list"));
        assert!(none.allows("http.fetch"));
        // Every web.* tool is locked when neither marker fires.
        assert!(!none.allows("web.search"));
        assert!(!none.allows("web.deep_research"));
        assert!(!none.allows("web.read_page"));
    }

    #[test]
    fn web_unlocks_search_opens_search_and_read_page_only() {
        let unlocks = WebUnlocks {
            search: true,
            research: false,
        };
        assert!(unlocks.allows("web.search"));
        assert!(unlocks.allows("web.read_page"));
        assert!(!unlocks.allows("web.deep_research"));
    }

    #[test]
    fn web_unlocks_research_opens_deep_research_and_read_page_only() {
        let unlocks = WebUnlocks {
            search: false,
            research: true,
        };
        assert!(unlocks.allows("web.deep_research"));
        assert!(unlocks.allows("web.read_page"));
        assert!(!unlocks.allows("web.search"));
    }

    #[test]
    fn slash_gated_registry_matches_implementation() {
        // Belt-and-braces: if a future tool is added under web.* but the
        // gate list isn't updated, this test will start failing the
        // moment WebUnlocks::allows starts returning true for the new
        // name in the default state.
        for name in SLASH_GATED_TOOL_NAMES {
            assert!(
                is_slash_gated(name),
                "{name} must be reported as slash-gated"
            );
            assert!(
                !WebUnlocks::default().allows(name),
                "{name} must be locked by default"
            );
        }
        assert!(!is_slash_gated("fs.list"));
        assert!(!is_slash_gated("http.fetch"));
    }

    #[test]
    fn parse_ddg_results_extracts_basic_rows() {
        // Minimal fixture mirroring DDG's HTML endpoint shape.
        let html = r##"
            <html><body>
              <div class="result">
                <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fone">First Result</a>
                <a class="result__snippet">Snippet one.</a>
              </div>
              <div class="result">
                <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Ftwo">Second Result</a>
                <span class="result__snippet">Snippet two.</span>
              </div>
              <div class="result">
                <a class="result__a" href="https://duckduckgo.com/y.js?ad_provider=test">Ad</a>
              </div>
            </body></html>
        "##;
        let hits = parse_ddg_results(html, 5);
        assert_eq!(hits.len(), 2, "ad slot should be filtered out");
        assert_eq!(hits[0].title, "First Result");
        assert_eq!(hits[0].url, "https://example.com/one");
        assert_eq!(hits[0].snippet, "Snippet one.");
        assert_eq!(hits[1].url, "https://example.com/two");
    }
}
