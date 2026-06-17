import { useEffect, useMemo, useState } from "react";
import { Panel } from "@/components/tui/Panel";
import { TuiInput, TuiTextarea } from "@/components/tui/Input";
import { TuiButton } from "@/components/tui/Button";
import { Spinner } from "@/components/tui/Spinner";
import { useMcpStore, type McpToolSchema } from "@/stores/mcp";
import { useSettingsStore, type McpServerConfig } from "@/stores/settings";

const EMPTY_DISABLED_BUILTINS: string[] = [];

const SELECT_CLASS =
  "w-full rounded-[4px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1 text-[12px] text-tui-fg outline-none transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] focus:border-b-2 focus:border-b-tui-accent";

/**
 * Tools page.
 *
 * Two stacked surfaces:
 *
 * 1. **Built-in tools** — populated from `mcp_list_builtins()`. These run
 *    in-process under the synthetic `builtin` MCP server id, are always
 *    available to the chat runner, and are read-only here. We group them
 *    by tool-name prefix (`fs.`, `http.`, …) and let the user expand a
 *    row to inspect the JSON-schema the model sees.
 *
 * 2. **MCP servers** — user-configured HTTP / SSE / stdio endpoints. Each
 *    card can be probed (`tools/list`), enabled/disabled, edited, or
 *    deleted. The add/edit form opens in a modal dialog (same pattern as
 *    the New-task dialog on the Tasks page).
 *
 * The chat runner picks up both surfaces via `mcp::catalog::fetch_enabled`,
 * so anything visible here is callable by the model in the next turn
 * (unless the user explicitly disabled it for that chat in the chat-header
 * tools popover).
 */
export function ToolsView() {
  const servers = useMcpStore((s) => s.servers);
  const probes = useMcpStore((s) => s.probes);
  const builtins = useMcpStore((s) => s.builtins);
  const list = useMcpStore((s) => s.list);
  const listBuiltins = useMcpStore((s) => s.listBuiltins);
  const upsert = useMcpStore((s) => s.upsert);
  const remove = useMcpStore((s) => s.remove);
  const setEnabled = useMcpStore((s) => s.setEnabled);
  const probe = useMcpStore((s) => s.probe);

  const [editing, setEditing] = useState<McpServerConfig | null>(null);
  const [builtinQuery, setBuiltinQuery] = useState("");

  const disabledBuiltins = useSettingsStore(
    (s) => s.builtin_tools_disabled ?? EMPTY_DISABLED_BUILTINS,
  );
  const saveSettings = useSettingsStore((s) => s.save);

  async function setBuiltinEnabled(name: string, enabled: boolean) {
    const current = disabledBuiltins;
    const next = enabled
      ? current.filter((n) => n !== name)
      : current.includes(name)
        ? current
        : [...current, name];
    if (next === current) return;
    await saveSettings({ builtin_tools_disabled: next });
  }

  useEffect(() => {
    void list();
    void listBuiltins();
  }, [list, listBuiltins]);

  function startNew() {
    setEditing({
      id: `mcp-${Date.now().toString(36)}`,
      name: "",
      transport: "http",
      url: "",
      headers: [],
      command: "",
      args: [],
      env: [],
      enabled: true,
    });
  }

  async function save() {
    if (!editing) return;
    if (!editing.id.trim()) return;
    if (editing.transport === "stdio") {
      if (!(editing.command ?? "").trim()) return;
    } else if (!editing.url.trim()) {
      return;
    }
    await upsert(editing);
    setEditing(null);
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3">
      <BuiltinsPanel
        tools={builtins}
        query={builtinQuery}
        onQuery={setBuiltinQuery}
        disabledNames={disabledBuiltins}
        onToggle={(name, enabled) => void setBuiltinEnabled(name, enabled)}
      />

      <Panel
        title="MCP servers"
        hint={`${servers.length} configured`}
        className="min-h-[220px] flex-1"
        flush
      >
        <div className="flex items-center justify-between gap-2 border-b border-tui-border p-3">
          <span className="text-[11px] text-tui-fg-muted">
            External Model Context Protocol endpoints (HTTP / SSE / stdio).
          </span>
          <TuiButton variant="primary" onClick={startNew}>
            + Add server
          </TuiButton>
        </div>
        <ul className="flex-1 overflow-auto">
          {servers.length === 0 && (
            <li className="px-4 py-4 text-[12px] text-tui-fg-muted">
              No MCP servers configured. Add one to expose its tools to zero —
              we send standard JSON-RPC{" "}
              <code className="font-mono">tools/list</code> +{" "}
              <code className="font-mono">tools/call</code> requests, so any
              MCP-compliant server (HTTP, SSE, or stdio) works.
            </li>
          )}
          {servers.map((srv) => {
            const probeState = probes[srv.id];
            return (
              <li
                key={srv.id}
                className="border-b border-tui-border px-3 py-2 text-[12px]"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-medium text-tui-fg">
                        {srv.name || srv.id}
                      </span>
                      <span className="rounded-[3px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-1.5 py-px text-[10px] text-tui-fg-muted">
                        {srv.transport}
                      </span>
                      {!srv.enabled && (
                        <span className="rounded-[3px] border border-tui-warn/40 bg-[rgba(252,225,0,0.10)] px-1.5 py-px text-[10px] text-tui-warn">
                          Disabled
                        </span>
                      )}
                    </div>
                    <div
                      className="truncate text-[11px] text-tui-fg-dim"
                      title={
                        srv.transport === "stdio" ? stdioTitle(srv) : srv.url
                      }
                    >
                      {srv.transport === "stdio" ? stdioTitle(srv) : srv.url}
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <label className="flex items-center gap-1.5 text-[11px] text-tui-fg-dim">
                      <input
                        type="checkbox"
                        checked={srv.enabled}
                        onChange={(e) =>
                          void setEnabled(srv.id, e.target.checked)
                        }
                      />
                      On
                    </label>
                    <TuiButton onClick={() => void probe(srv.id)}>
                      {probeState?.loading ? <Spinner size="sm" /> : "Probe"}
                    </TuiButton>
                    <TuiButton onClick={() => setEditing({ ...srv })}>
                      Edit
                    </TuiButton>
                    <TuiButton
                      variant="danger"
                      onClick={() => {
                        if (
                          confirm(`Remove MCP server "${srv.name || srv.id}"?`)
                        )
                          void remove(srv.id);
                      }}
                    >
                      Delete
                    </TuiButton>
                  </div>
                </div>
                {probeState?.error && (
                  <div className="mt-1.5 rounded border border-tui-err/30 bg-[rgba(255,153,164,0.06)] px-2 py-1 text-[11px] text-tui-err">
                    {probeState.error}
                  </div>
                )}
                {probeState?.tools && probeState.tools.length > 0 && (
                  <ul className="mt-2 space-y-1 rounded-md border border-tui-border bg-[var(--fluent-bg-subtle)] px-2 py-1.5 text-[11px]">
                    {probeState.tools.map((t) => (
                      <li
                        key={t.name}
                        className="flex items-baseline gap-2"
                        title={t.description}
                      >
                        <span className="font-medium text-tui-accent">
                          {t.name}
                        </span>
                        {t.destructive && (
                          <span className="rounded-[3px] border border-tui-warn/40 bg-[rgba(252,225,0,0.10)] px-1 text-[9px] text-tui-warn">
                            destructive
                          </span>
                        )}
                        <span className="truncate text-[11px] text-tui-fg-muted">
                          {t.description}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      </Panel>

      {editing && (
        <McpEditorDialog
          value={editing}
          onChange={setEditing}
          onCancel={() => setEditing(null)}
          onSave={() => void save()}
        />
      )}
    </div>
  );
}

/* ─── Built-in tools panel ──────────────────────────────────────────── */

/**
 * Grouped, searchable list of built-in tools. The user can expand a row
 * to inspect the JSON-schema the model receives. We group by the dotted
 * name prefix (`fs.list` → "fs"); tools without a prefix go under
 * "other".
 */
function BuiltinsPanel({
  tools,
  query,
  onQuery,
  disabledNames,
  onToggle,
}: {
  tools: McpToolSchema[];
  query: string;
  onQuery: (q: string) => void;
  disabledNames: string[];
  onToggle: (name: string, enabled: boolean) => void;
}) {
  const disabledSet = useMemo(() => new Set(disabledNames), [disabledNames]);
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return tools;
    return tools.filter(
      (t) =>
        t.name.toLowerCase().includes(q) ||
        t.description.toLowerCase().includes(q),
    );
  }, [tools, query]);

  const grouped = useMemo(() => groupByPrefix(filtered), [filtered]);
  const enabledCount = tools.length - disabledSet.size;

  return (
    <Panel
      title="Built-in tools"
      hint={`${enabledCount}/${tools.length} on · always available`}
      className="max-h-[55%] min-h-[180px] shrink-0"
      flush
    >
      <div className="flex items-center justify-between gap-2 border-b border-tui-border p-3">
        <span className="text-[11px] text-tui-fg-muted">
          In-process tools the agent can call without a network round-trip.
          Toggling here applies to <strong>all</strong> chats; use the chat-
          header tools popover to disable a tool for a single conversation.
        </span>
        <div className="w-56 shrink-0">
          <TuiInput
            value={query}
            onChange={(e) => onQuery(e.target.value)}
            placeholder="Filter tools…"
            aria-label="Filter built-in tools"
          />
        </div>
      </div>
      <div className="flex-1 overflow-auto">
        {tools.length === 0 ? (
          <div className="px-3 py-4 text-[12px] text-tui-fg-muted">
            No built-in tools registered.
          </div>
        ) : filtered.length === 0 ? (
          <div className="px-3 py-4 text-[12px] text-tui-fg-muted">
            No built-ins match "{query}".
          </div>
        ) : (
          <ul>
            {grouped.map(([prefix, list]) => {
              const disabledInGroup = list.reduce(
                (n, t) => (disabledSet.has(t.name) ? n + 1 : n),
                0,
              );
              const enabledInGroup = list.length - disabledInGroup;
              return (
                <li key={prefix} className="border-b border-tui-border">
                  <details className="group/cat" open={!!query.trim()}>
                    <summary className="flex cursor-pointer select-none items-baseline gap-2 bg-[rgba(255,255,255,0.022)] px-3 py-1.5 outline-none hover:bg-[rgba(255,255,255,0.04)]">
                      <span className="text-tui-fg-muted transition-transform group-open/cat:rotate-90">
                        ▸
                      </span>
                      <span className="text-[11px] font-semibold uppercase tracking-wide text-tui-fg-dim">
                        {prefix}
                      </span>
                      <span className="text-[10px] text-tui-fg-muted">
                        {enabledInGroup}/{list.length} on
                      </span>
                    </summary>
                    <ul>
                      {list.map((t) => (
                        <BuiltinRow
                          key={t.name}
                          tool={t}
                          disabled={disabledSet.has(t.name)}
                          onToggle={(enabled) => onToggle(t.name, enabled)}
                        />
                      ))}
                    </ul>
                  </details>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </Panel>
  );
}

function BuiltinRow({
  tool,
  disabled,
  onToggle,
}: {
  tool: McpToolSchema;
  disabled: boolean;
  onToggle: (enabled: boolean) => void;
}) {
  const schemaJson = useMemo(() => {
    try {
      return JSON.stringify(tool.input_schema, null, 2);
    } catch {
      return String(tool.input_schema ?? "");
    }
  }, [tool.input_schema]);

  return (
    <li
      className={
        "border-t border-tui-border px-3 py-2 text-[12px]" +
        (disabled ? " opacity-60" : "")
      }
    >
      <details className="group">
        <summary className="flex cursor-pointer select-none items-center gap-2 outline-none">
          <span className="text-tui-fg-muted transition-transform group-open:rotate-90">
            ▸
          </span>
          <span className="font-medium text-tui-accent">{tool.name}</span>
          {tool.destructive && (
            <span className="rounded-[3px] border border-tui-warn/40 bg-[rgba(252,225,0,0.10)] px-1 text-[9px] text-tui-warn">
              destructive
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-[11px] text-tui-fg-dim">
            {tool.description}
          </span>
          <span
            className="shrink-0"
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
            }}
          >
            <TuiButton
              variant={disabled ? "primary" : "danger"}
              onClick={() => onToggle(disabled)}
              aria-label={`${disabled ? "Enable" : "Disable"} ${tool.name} for all chats`}
            >
              {disabled ? "Enable" : "Disable"}
            </TuiButton>
          </span>
        </summary>
        <div className="mt-2 ml-5 space-y-2">
          <div className="text-[11px] text-tui-fg-dim">{tool.description}</div>
          <details className="rounded border border-tui-border bg-[var(--fluent-bg-subtle)] text-[11px]">
            <summary className="cursor-pointer select-none px-2 py-1 text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)]">
              Input schema
            </summary>
            <pre className="max-h-64 overflow-auto whitespace-pre-wrap border-t border-tui-border px-2 py-1.5 font-mono text-tui-fg-dim">
              {schemaJson}
            </pre>
          </details>
        </div>
      </details>
    </li>
  );
}

function groupByPrefix(tools: McpToolSchema[]): [string, McpToolSchema[]][] {
  const groups = new Map<string, McpToolSchema[]>();
  for (const t of tools) {
    const idx = t.name.indexOf(".");
    const prefix = idx > 0 ? t.name.slice(0, idx) : "other";
    const bucket = groups.get(prefix);
    if (bucket) bucket.push(t);
    else groups.set(prefix, [t]);
  }
  const out = Array.from(groups.entries());
  out.sort((a, b) => {
    if (a[0] === "other") return 1;
    if (b[0] === "other") return -1;
    return a[0].localeCompare(b[0]);
  });
  for (const [, list] of out) {
    list.sort((a, b) => a.name.localeCompare(b.name));
  }
  return out;
}

/* ─── New/Edit MCP server dialog ────────────────────────────────────── */

function McpEditorDialog({
  value,
  onChange,
  onCancel,
  onSave,
}: {
  value: McpServerConfig;
  onChange: (next: McpServerConfig) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onCancel();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const headers = value.headers ?? [];
  const args = value.args ?? [];
  const env = value.env ?? [];
  const isStdio = value.transport === "stdio";

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-16"
      role="presentation"
      onClick={onCancel}
    >
      <div
        className="flex max-h-[88vh] w-[640px] max-w-[94vw] flex-col overflow-hidden rounded-xl border border-tui-border bg-tui-bg-elev shadow-[var(--fluent-shadow-16)]"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="mcp-editor-title"
      >
        <header className="flex items-center justify-between gap-3 border-b border-tui-border px-4 py-3">
          <div className="min-w-0">
            <div
              id="mcp-editor-title"
              className="text-[14px] font-semibold text-tui-fg"
            >
              {value.name || value.id ? "Edit MCP server" : "New MCP server"}
            </div>
            <div className="truncate text-[11px] text-tui-fg-muted">
              Expose an external Model Context Protocol endpoint to zero.
            </div>
          </div>
          <button
            onClick={onCancel}
            className="flex h-7 w-7 items-center justify-center rounded text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
            title="Close"
            aria-label="Close"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            >
              <path d="M18 6 6 18M6 6l12 12" />
            </svg>
          </button>
        </header>

        <div className="flex-1 space-y-3 overflow-auto p-4 text-[12px]">
          <div className="grid grid-cols-2 gap-3">
            <McpField label="ID">
              <TuiInput
                value={value.id}
                onChange={(e) =>
                  onChange({ ...value, id: e.target.value.trim() })
                }
              />
            </McpField>
            <McpField label="Name">
              <TuiInput
                value={value.name}
                onChange={(e) => onChange({ ...value, name: e.target.value })}
                placeholder="My MCP server"
              />
            </McpField>
          </div>

          <McpField label="Transport">
            <select
              value={value.transport}
              onChange={(e) =>
                onChange({ ...value, transport: e.target.value })
              }
              className={SELECT_CLASS}
            >
              <option value="http">HTTP (streamable)</option>
              <option value="sse">SSE</option>
              <option value="stdio">stdio (spawn a child process)</option>
            </select>
          </McpField>

          {isStdio ? (
            <>
              <McpField label="Command">
                <TuiInput
                  value={value.command ?? ""}
                  onChange={(e) =>
                    onChange({ ...value, command: e.target.value })
                  }
                  placeholder="uvx, npx, or absolute path to a binary"
                />
              </McpField>
              <McpField label="Arguments (one per line)">
                <TuiTextarea
                  rows={4}
                  value={args.join("\n")}
                  onChange={(e) =>
                    onChange({
                      ...value,
                      args: e.target.value
                        .split("\n")
                        .map((s) => s.trim())
                        .filter((s) => s.length > 0),
                    })
                  }
                  placeholder={"mcp-server-fetch"}
                />
              </McpField>
              <EnvList
                env={env}
                onChange={(next) => onChange({ ...value, env: next })}
              />
            </>
          ) : (
            <>
              <McpField label="URL">
                <TuiInput
                  value={value.url}
                  onChange={(e) => onChange({ ...value, url: e.target.value })}
                  placeholder="http://127.0.0.1:9001/mcp"
                />
              </McpField>
              <HeaderList
                headers={headers}
                onChange={(next) => onChange({ ...value, headers: next })}
              />
            </>
          )}

          <label className="flex items-center gap-2 pt-1 text-[12px] text-tui-fg-dim">
            <input
              type="checkbox"
              checked={value.enabled}
              onChange={(e) =>
                onChange({ ...value, enabled: e.target.checked })
              }
            />
            Enabled (advertise to the chat runner)
          </label>
        </div>

        <footer className="flex items-center justify-end gap-2 border-t border-tui-border px-4 py-3">
          <TuiButton onClick={onCancel}>Cancel</TuiButton>
          <TuiButton variant="primary" onClick={onSave}>
            Save
          </TuiButton>
        </footer>
      </div>
    </div>
  );
}

function HeaderList({
  headers,
  onChange,
}: {
  headers: [string, string][];
  onChange: (next: [string, string][]) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5 text-[11px] text-tui-fg-dim">
      <span className="font-medium">Headers</span>
      {headers.map(([k, v], i) => (
        <div key={i} className="flex gap-1.5">
          <TuiInput
            value={k}
            onChange={(e) => {
              const next = headers.slice();
              next[i] = [e.target.value, v];
              onChange(next);
            }}
            placeholder="Authorization"
          />
          <TuiInput
            value={v}
            onChange={(e) => {
              const next = headers.slice();
              next[i] = [k, e.target.value];
              onChange(next);
            }}
            placeholder="Bearer …"
          />
          <TuiButton
            variant="danger"
            onClick={() => onChange(headers.filter((_, j) => j !== i))}
            aria-label="Remove header"
          >
            <CloseGlyph />
          </TuiButton>
        </div>
      ))}
      <TuiButton onClick={() => onChange([...headers, ["", ""]])}>
        + Add header
      </TuiButton>
    </div>
  );
}

function EnvList({
  env,
  onChange,
}: {
  env: [string, string][];
  onChange: (next: [string, string][]) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5 text-[11px] text-tui-fg-dim">
      <span className="font-medium">Environment variables</span>
      {env.map(([k, v], i) => (
        <div key={i} className="flex gap-1.5">
          <TuiInput
            value={k}
            onChange={(e) => {
              const next = env.slice();
              next[i] = [e.target.value, v];
              onChange(next);
            }}
            placeholder="MY_API_KEY"
          />
          <TuiInput
            value={v}
            onChange={(e) => {
              const next = env.slice();
              next[i] = [k, e.target.value];
              onChange(next);
            }}
            placeholder="…"
          />
          <TuiButton
            variant="danger"
            onClick={() => onChange(env.filter((_, j) => j !== i))}
            aria-label="Remove env var"
          >
            <CloseGlyph />
          </TuiButton>
        </div>
      ))}
      <TuiButton onClick={() => onChange([...env, ["", ""]])}>
        + Add variable
      </TuiButton>
    </div>
  );
}

function CloseGlyph() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    >
      <path d="M18 6 6 18M6 6l12 12" />
    </svg>
  );
}

function McpField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1 text-[11px] font-medium text-tui-fg-dim">
      {label}
      {children}
    </label>
  );
}

function stdioTitle(srv: McpServerConfig): string {
  const parts = [srv.command ?? "", ...(srv.args ?? [])].filter((s) =>
    s.trim(),
  );
  return parts.length > 0 ? parts.join(" ") : "(stdio: no command set)";
}
