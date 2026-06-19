# ZerØ

A fast, simple **local** agentic AI desktop app. Built for low-end Intel laptops first, scales up to higher-end machines.

- **Runtime:** [OpenVINO Model Server](https://docs.openvino.ai/2026/index.html) (python-free `ovms.exe`)
- **Future runtimes:** ollama, llama.cpp (same OpenAI-compatible interface)
- **UI:** Tauri 2 + React + Tailwind v4 (TUI-style)
- **State:** Zustand on the front, SQLite (`sqlx` + `sqlite-vec`) on the back
- **Storage root:** `~/.zero/` (same path on Windows, macOS, and Linux)

## Features (planned phases)

1. **MVP (this scaffold):** shell + system probe + OVMS lifecycle + HF model browse/download + streaming chat + conversation persistence
2. **Multimodal chat + extensibility:** image / document upload (OpenAI vision shape over the OVMS chat endpoint), user-authored skills (`~/.zero/skills/<id>/SKILL.md`), and external MCP servers (HTTP/SSE) listed + smoke-tested from the Tools page
3. Built-in MCP tools (shell, fs, http, clipboard, notify, task.create) + agent loop wiring them in
4. Memory (short-term summary + long-term vector recall)
5. Agent loop (ReAct, tool calls, safety caps)
6. Scheduler (cron + interval + manual click-to-run agents)

## Getting started

```bash
pnpm install
pnpm tauri dev
```

### Icons (required for `tauri build`, not for `dev`)

```bash
pnpm tauri icon path/to/your-icon.png
```

## Layout

| Path | Purpose |
| --- | --- |
| `src/` | React frontend (TUI-style) |
| `src/stores/` | Zustand stores (one per domain) |
| `src/components/tui/` | Reusable TUI primitives |
| `src/pages/` | Top-level views |
| `src-tauri/src/` | Rust backend (one module per domain) |
| `src-tauri/src/commands/` | Tauri IPC commands |

## Storage

```
~/.zero/
├── zero.db                 # SQLite (conversations, tasks, memory, vectors)
├── system.json             # Cached hardware probe
├── runtimes/ovms/          # Downloaded ovms.exe + deps
├── models/                 # HF model cache (OpenVINO IR)
├── attachments/<conv_id>/  # Persisted chat uploads (images + docs)
├── skills/<skill_id>/      # SKILL.md + supporting resources
├── logs/
└── settings.json
```

On Windows that resolves to `C:\Users\<you>\.zero\`. Older installs under
`%LOCALAPPDATA%\zero\zero\data\` are moved into `~/.zero` automatically on
first launch.
