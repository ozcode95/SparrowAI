//! Shared `Settings` model.
//!
//! Lives in its own module (rather than under `commands::settings`) so the
//! agent runner and provider factory can read settings without going through
//! Tauri's IPC layer. The actual `#[tauri::command]` wrappers in
//! `commands::settings` delegate here.
//!
//! Storage is intentionally trivial right now (one JSON blob at
//! `<root>/settings.json`). Schema evolution is `serde(default)`-driven so old
//! files keep working as we add fields.

use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: String,
    /// Where the provider lives: `local` (a runtime on this machine) vs.
    /// `cloud` (a remote hosted endpoint). Free-form string so additional
    /// classifications can be added later without breaking older settings
    /// files. Defaults to `local` for entries written before this field
    /// existed.
    #[serde(default = "default_provider_location")]
    pub location: String,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    #[serde(default)]
    pub api_key_ref: Option<String>,
    /// Per-provider sampling override. Empty fields mean "use the
    /// per-model [`crate::chat::runner::ModelProfile`] default". Layered
    /// under any per-conversation override so a single chat can still
    /// diverge from the provider-wide value.
    #[serde(default)]
    pub sampling: SamplingConfig,
}

fn default_provider_location() -> String {
    "local".into()
}

/// Optional sampling overrides. Every field is independently optional:
/// a `None` means "don't override", letting the precedence chain
/// (conversation → provider → model profile → upstream default) fall
/// through to the next layer.
///
/// Stored on [`ProviderConfig`] (global) and on each conversation row
/// (per-chat). Identical shape on the wire so the UI can reuse one
/// editor component for both surfaces.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SamplingConfig {
    /// Override the sampling temperature. Typical range 0.0–2.0.
    pub temperature: Option<f32>,
    /// Override the nucleus-sampling cutoff. Typical range 0.0–1.0.
    pub top_p: Option<f32>,
    /// Override the top-k cutoff. Forwarded as an OpenAI-compat
    /// extension; ignored by servers that don't understand it.
    pub top_k: Option<u32>,
}

/// User-configured external MCP server. Supports three transports:
///
/// - `http` / `sse` — JSON-RPC over the streamable HTTP transport. `url`
///   + `headers` are used; `command` / `args` / `env` are ignored.
/// - `stdio`        — zero spawns `command` with `args` (and any extra
///   `env`) and speaks JSON-RPC on the child's stdin/stdout per call.
///   `url` and `headers` are ignored.
///
/// All transport-specific fields carry `#[serde(default)]` so older
/// settings files (which only had `url` + `headers`) keep loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    /// `http` | `sse` | `stdio`. Free-form so additional transports can
    /// be added without breaking existing settings files.
    #[serde(default = "default_mcp_transport")]
    pub transport: String,
    /// Base URL for the MCP endpoint, e.g. `http://127.0.0.1:9001/mcp`.
    /// Empty for stdio transport.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// For `stdio` transport: the executable to spawn (e.g. `uvx`,
    /// `npx`, an absolute path to a binary). Unused for HTTP/SSE.
    #[serde(default)]
    pub command: String,
    /// For `stdio` transport: argv to pass to `command`.
    #[serde(default)]
    pub args: Vec<String>,
    /// For `stdio` transport: extra environment variables to set on the
    /// child (merged on top of the zero process env).
    #[serde(default)]
    pub env: Vec<(String, String)>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_mcp_transport() -> String {
    "http".into()
}

/// User-visible OVMS runtime knobs. Stored alongside the rest of `Settings`
/// so the UI can edit them and `OvmsController::start` can read them at spawn
/// time without going through Tauri commands.
///
/// New optional fields all carry `#[serde(default)]` so older `settings.json`
/// files keep loading after upgrades. See
/// <https://docs.openvino.ai/2026/model-server/ovms_docs_parameters.html>
/// for the upstream CLI flags these map to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvmsSettings {
    pub rest_port: u16,
    /// gRPC bind port. `0` disables the gRPC server entirely (OVMS's
    /// documented way to opt out). zero talks to OVMS over REST only, so
    /// the default is `0` to avoid pointlessly grabbing a port (and the
    /// `WSAEADDRINUSE` failures that come with it when something else is
    /// already bound on 9000). Users who need gRPC for other clients can
    /// set this to a free port from the Settings UI.
    pub grpc_port: u16,
    /// One of `CPU`, `GPU`, `NPU`, `AUTO`. Free-form so future OpenVINO
    /// devices (`MULTI:CPU,GPU`, `HETERO:GPU,CPU`, ...) can be entered directly.
    pub device: String,

    // ─── advanced server-side knobs (all optional) ─────────────────────
    /// `DEBUG` / `INFO` / `ERROR`. `None` keeps the OVMS default (INFO).
    /// Mapped to `--log_level`.
    #[serde(default)]
    pub log_level: Option<String>,
    /// Optional path to a log file. Mapped to `--log_path`.
    #[serde(default)]
    pub log_path: Option<String>,
    /// Optional model compilation cache directory. Big startup speedup on
    /// subsequent runs. Mapped to `--cache_dir`.
    #[serde(default)]
    pub cache_dir: Option<String>,
    /// CORS `Access-Control-Allow-Origin`. Mapped to `--allowed_origins`.
    #[serde(default)]
    pub allowed_origins: Option<String>,
    /// Path to a file whose first line holds the API key required on
    /// `/v3/*` endpoints. Mapped to `--api_key_file`.
    #[serde(default)]
    pub api_key_file: Option<String>,
    /// Escape hatch for any flag we don't model explicitly. Each entry is
    /// forwarded verbatim as a separate `argv` token — the UI splits the
    /// user's textarea on newlines so multi-token flags become two entries
    /// (`--grpc_workers` then `4`).
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl Default for OvmsSettings {
    fn default() -> Self {
        Self {
            rest_port: 8000,
            grpc_port: 0,
            device: "GPU".into(),
            log_level: None,
            log_path: None,
            cache_dir: None,
            allowed_origins: None,
            api_key_file: None,
            extra_args: Vec::new(),
        }
    }
}

impl OvmsSettings {
    /// Canonical OpenAI-compatible base URL derived from the configured REST
    /// port. Kept here (rather than in `OvmsController`) so the same string is
    /// always used by the provider, the info struct, and the readiness probe.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v3", self.rest_port)
    }

    /// Render the always-on + optional knobs into an `argv` vector ready to
    /// hand to `ovms.exe`. `--config_path` is **not** included here because
    /// it's resolved per-start by the controller (it depends on which model
    /// is being loaded). Callers should append it after this.
    pub fn cli_args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--port".into(),
            self.grpc_port.to_string(),
            "--rest_port".into(),
            self.rest_port.to_string(),
        ];
        push_opt(&mut args, "--log_level", self.log_level.as_deref());
        push_opt(&mut args, "--log_path", self.log_path.as_deref());
        push_opt(&mut args, "--cache_dir", self.cache_dir.as_deref());
        push_opt(
            &mut args,
            "--allowed_origins",
            self.allowed_origins.as_deref(),
        );
        push_opt(&mut args, "--api_key_file", self.api_key_file.as_deref());
        for raw in &self.extra_args {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }
        args
    }
}

/// Helper: push a `--flag value` pair iff `value` is `Some(non-empty)`.
fn push_opt(args: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(v) = value {
        let v = v.trim();
        if !v.is_empty() {
            args.push(flag.to_string());
            args.push(v.to_string());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub active_provider_id: Option<String>,
    pub providers: Vec<ProviderConfig>,
    pub hf_token_set: bool,
    pub default_model: Option<String>,
    pub thinking_enabled: bool,
    pub agent_max_iterations: u32,
    pub destructive_tool_confirm: bool,
    #[serde(default)]
    pub ovms: OvmsSettings,
    /// When true (the default), startup will install the OVMS binary if
    /// missing and auto-start it with `default_model` (when that model is
    /// already downloaded locally). Users who want to manage OVMS by hand
    /// can flip this off from the settings page.
    #[serde(default = "default_true")]
    pub auto_provision_ovms: bool,
    /// Ids of skills (folder names under `~/.zero/skills/`) that should be
    /// injected into the system prompt for every new turn. Skills that no
    /// longer exist on disk are silently ignored.
    #[serde(default)]
    pub skills_enabled: Vec<String>,
    /// Names of built-in tools (e.g. `fs.list`, `shell.exec`) that the user
    /// has globally disabled from the Tools page. The chat catalog hides
    /// these across every conversation; the per-chat tools popover only
    /// disables a tool for one conversation, which is a separate override.
    #[serde(default)]
    pub builtin_tools_disabled: Vec<String>,
    /// When true, the chat runner ships only the `tools.list` built-in to
    /// the LLM on the first round of each turn instead of the full tool
    /// catalogue. The model must call `tools.list` to discover what else
    /// is available; once it does, the runner expands the OpenAI `tools`
    /// array on subsequent rounds in the same turn to expose everything.
    /// Trades a single extra round-trip on tool-using turns for a much
    /// smaller initial-context cost — valuable when many MCP servers are
    /// wired up but most turns don't actually need a tool. Defaults on so
    /// new chats favour smaller initial context; the safety-net check in
    /// the runner still tolerates a hand-edited `false` for users who
    /// want every tool inlined up front.
    #[serde(default = "default_true")]
    pub lazy_tool_discovery: bool,
    /// External MCP server configurations.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_provider_id: Some("local".into()),
            providers: vec![ProviderConfig {
                id: "local".into(),
                kind: "ovms".into(),
                location: "local".into(),
                name: "Local".into(),
                base_url: "http://127.0.0.1:8000/v3".into(),
                enabled: true,
                api_key_ref: None,
                sampling: SamplingConfig::default(),
            }],
            hf_token_set: false,
            default_model: None,
            thinking_enabled: true,
            agent_max_iterations: 8,
            destructive_tool_confirm: true,
            ovms: OvmsSettings::default(),
            auto_provision_ovms: true,
            skills_enabled: Vec::new(),
            // Conservative built-in surface for fresh installs: filesystem
            // mutation (`fs.edit` / `fs.write`), metadata probing
            // (`fs.stat`), arbitrary outbound HTTP (`http.fetch`), and
            // clipboard I/O start disabled so the agent can't surprise a
            // brand-new user. Each one can be flipped back on from the
            // Tools page.
            builtin_tools_disabled: vec![
                "clipboard.read".into(),
                "clipboard.write".into(),
                "http.fetch".into(),
                "fs.edit".into(),
                "fs.write".into(),
                "fs.stat".into(),
            ],
            lazy_tool_discovery: true,
            mcp_servers: Vec::new(),
        }
    }
}

impl Settings {
    /// Reads the on-disk settings file, falling back to `Default` on a missing
    /// or unparsable file. We deliberately don't error here so a corrupt file
    /// never bricks the app — the user can re-save from the UI to fix it.
    pub async fn load() -> Result<Self> {
        let p = paths::settings_file()?;
        if !p.exists() {
            return Ok(Self::default());
        }
        let bytes = tokio::fs::read(&p).await?;
        let mut s: Self = serde_json::from_slice(&bytes).unwrap_or_else(|_| Self::default());
        s.migrate_legacy_defaults();
        Ok(s)
    }

    /// In-place rewrites for fields whose default changed in a way that
    /// would otherwise leave existing installs stuck on the old value.
    /// Each branch is keyed off the *exact* previous default so users who
    /// explicitly set a non-default value are never overridden.
    fn migrate_legacy_defaults(&mut self) {
        // gRPC default flipped from 9000 → 0 (disabled) once we confirmed
        // zero never talks to OVMS over gRPC. The old default routinely
        // collided with stray processes on Windows (WSAEADDRINUSE) and
        // kept OVMS from starting at all. Users who actively want gRPC
        // can re-enter a port from the Settings page.
        if self.ovms.grpc_port == 9000 {
            self.ovms.grpc_port = 0;
        }
        // `builtin_tools_disabled` previously defaulted to an empty list
        // — every built-in tool was advertised on a fresh install. We
        // now ship a conservative subset (clipboard / http.fetch / fs
        // mutation + stat) disabled by default. An exactly-empty list on
        // disk almost certainly came from that old default rather than
        // an explicit user choice (“I want every tool on” is far rarer
        // than “I never touched the Tools page” on a tool already gated
        // behind an opt-in confirm prompt), so upgrade it in place.
        if self.builtin_tools_disabled.is_empty() {
            self.builtin_tools_disabled = Self::default().builtin_tools_disabled;
        }
    }

    pub async fn save(&self) -> Result<()> {
        let p = paths::settings_file()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(&p, bytes).await?;
        Ok(())
    }

    /// Returns the active provider config, falling back to the first enabled
    /// provider if `active_provider_id` is unset or stale.
    pub fn active_provider(&self) -> Option<&ProviderConfig> {
        if let Some(id) = self.active_provider_id.as_deref() {
            if let Some(p) = self.providers.iter().find(|p| p.id == id) {
                return Some(p);
            }
        }
        self.providers.iter().find(|p| p.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disables_grpc() {
        let s = OvmsSettings::default();
        assert_eq!(s.grpc_port, 0);
    }

    #[test]
    fn cli_args_pass_grpc_port_through_including_zero() {
        // `--port 0` is the documented OVMS way to disable the gRPC
        // server; the controller must forward it verbatim instead of
        // silently dropping the flag (which would make OVMS fall back to
        // its own default port).
        let s = OvmsSettings::default();
        let args = s.cli_args();
        let i = args
            .iter()
            .position(|a| a == "--port")
            .expect("--port flag");
        assert_eq!(args[i + 1], "0");
    }

    #[test]
    fn migrate_legacy_grpc_default_to_zero() {
        let mut s = Settings::default();
        s.ovms.grpc_port = 9000;
        s.migrate_legacy_defaults();
        assert_eq!(s.ovms.grpc_port, 0);
    }

    #[test]
    fn migrate_preserves_user_chosen_grpc_port() {
        let mut s = Settings::default();
        s.ovms.grpc_port = 9001;
        s.migrate_legacy_defaults();
        assert_eq!(s.ovms.grpc_port, 9001);
    }

    #[test]
    fn migrate_legacy_empty_builtin_disabled_to_safe_default() {
        // An exactly-empty list on disk maps to the old "every tool
        // enabled" default; the migration upgrades it to the current
        // conservative set so existing installs match a fresh one.
        let mut s = Settings::default();
        s.builtin_tools_disabled.clear();
        s.migrate_legacy_defaults();
        let want = Settings::default().builtin_tools_disabled;
        assert_eq!(s.builtin_tools_disabled, want);
        assert!(want.iter().any(|n| n == "clipboard.read"));
        assert!(want.iter().any(|n| n == "http.fetch"));
        assert!(want.iter().any(|n| n == "fs.write"));
    }

    #[test]
    fn migrate_preserves_user_chosen_builtin_disabled() {
        // Any non-empty list signals the user has touched the Tools
        // page; we leave their picks alone even if shorter / different
        // from the new default set.
        let mut s = Settings::default();
        s.builtin_tools_disabled = vec!["shell.exec".into()];
        s.migrate_legacy_defaults();
        assert_eq!(s.builtin_tools_disabled, vec!["shell.exec".to_string()]);
    }
}
