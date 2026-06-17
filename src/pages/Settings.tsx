import { useEffect, useState, type ReactNode } from "react";
import { TuiButton } from "@/components/tui/Button";
import { TuiInput, TuiSelect, TuiTextarea } from "@/components/tui/Input";
import { Toggle } from "@/components/tui/Toggle";
import { Spinner } from "@/components/tui/Spinner";
import { SamplingEditor } from "@/components/SamplingEditor";
import {
  useSettingsStore,
  EMPTY_SAMPLING,
  type OvmsSettings,
  type ProviderConfig,
  type ProviderKind,
  type SamplingConfig,
} from "@/stores/settings";
import {
  useServerStore,
  type LoadedModel,
  type TfsConfigStatus,
} from "@/stores/server";
import { COMPOUND_DEVICE_SEEDS, DEVICE_CHOICES } from "@/lib/modelParams";
import {
  formatSamplingDefault,
  modelSamplingDefaults,
} from "@/lib/modelProfile";
import {
  InstallProgressCard,
  OvmsStatusCard,
  SystemSpecsCard,
} from "@/components/server/ServerControls";

const PROVIDER_KINDS: ProviderKind[] = ["ovms", "ollama", "vllm"];

/** Sensible default `base_url` for a freshly added provider of `kind`. */
function defaultBaseUrl(kind: ProviderKind): string {
  switch (kind) {
    case "ovms":
      return "http://127.0.0.1:8000/v3";
    case "ollama":
      return "http://127.0.0.1:11434/v1";
    case "vllm":
      return "http://127.0.0.1:8000/v1";
  }
}

/** Generate a stable-ish id for a newly created provider row. */
function freshProviderId(existing: ProviderConfig[]): string {
  for (let i = 1; i < 1000; i++) {
    const id = `provider-${i}`;
    if (!existing.some((p) => p.id === id)) return id;
  }
  return `provider-${Date.now()}`;
}

// ─── section navigation ──────────────────────────────────────────────

type SectionId = "general" | "providers" | "server" | "system";

interface Section {
  id: SectionId;
  label: string;
  hint: string;
  icon: ReactNode;
}

const SECTIONS: Section[] = [
  {
    id: "general",
    label: "General",
    hint: "Agent behaviour and startup",
    icon: <SlidersIcon />,
  },
  {
    id: "providers",
    label: "Providers",
    hint: "OpenAI-compatible endpoints",
    icon: <PlugIcon />,
  },
  {
    id: "server",
    label: "OVMS server",
    hint: "Runtime, ports, device",
    icon: <ServerIcon />,
  },
  {
    id: "system",
    label: "System",
    hint: "Host hardware snapshot",
    icon: <ChipIcon />,
  },
];

// ─── page shell ──────────────────────────────────────────────────────

export function SettingsView() {
  const [section, setSection] = useState<SectionId>("general");

  return (
    <div className="flex min-h-0 min-w-0 flex-1 gap-4">
      {/* Section rail */}
      <nav
        className="w-[210px] shrink-0 self-start"
        aria-label="Settings sections"
      >
        <ul className="space-y-0.5">
          {SECTIONS.map((s) => {
            const active = s.id === section;
            return (
              <li key={s.id}>
                <button
                  onClick={() => setSection(s.id)}
                  aria-current={active ? "page" : undefined}
                  className={
                    "group relative flex w-full items-center gap-2.5 rounded-[6px] py-2 pl-3 pr-2.5 " +
                    "text-left text-[12px] transition-colors duration-150 " +
                    (active
                      ? "bg-[var(--fluent-bg-subtle-selected)] text-tui-fg"
                      : "text-tui-fg-dim hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg")
                  }
                >
                  {active && (
                    <span className="absolute left-0 top-1/2 h-[55%] w-[3px] -translate-y-1/2 rounded-full bg-tui-accent" />
                  )}
                  <span
                    className={
                      "flex h-5 w-5 items-center justify-center " +
                      (active ? "text-tui-accent" : "text-tui-fg-muted")
                    }
                    aria-hidden
                  >
                    {s.icon}
                  </span>
                  <span className="flex min-w-0 flex-col leading-tight">
                    <span className="truncate font-medium">{s.label}</span>
                    <span className="truncate text-[10.5px] text-tui-fg-muted">
                      {s.hint}
                    </span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </nav>

      {/* Section content */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto pr-1">
        <div className="mx-auto w-full max-w-[760px] space-y-4 pb-6">
          {section === "general" && <GeneralSection />}
          {section === "providers" && <ProvidersSection />}
          {section === "server" && <ServerSection />}
          {section === "system" && <SystemSection />}
        </div>
      </div>
    </div>
  );
}

// ─── reusable section primitives ────────────────────────────────────

/** Card with a small section title strip and any children stacked inside. */
function SettingsGroup({
  title,
  description,
  action,
  children,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="fluent-mica overflow-hidden rounded-[10px] border border-tui-border shadow-[var(--fluent-shadow-2)]">
      <header className="flex items-baseline justify-between gap-3 border-b border-tui-border bg-[rgba(255,255,255,0.022)] px-4 py-2.5">
        <div className="min-w-0">
          <h3 className="text-[12px] font-semibold text-tui-fg">{title}</h3>
          {description && (
            <p className="mt-0.5 text-[11px] text-tui-fg-muted">
              {description}
            </p>
          )}
        </div>
        {action && <div className="shrink-0">{action}</div>}
      </header>
      <div className="px-4 py-1.5">{children}</div>
    </section>
  );
}

/** Horizontal label/control row with hairline separators between siblings. */
function Field({
  label,
  hint,
  control,
  align = "center",
}: {
  label: string;
  hint?: ReactNode;
  control: ReactNode;
  align?: "center" | "start";
}) {
  return (
    <div
      className={
        "flex justify-between gap-6 border-b border-tui-border py-3 last:border-b-0 " +
        (align === "start" ? "items-start" : "items-center")
      }
    >
      <div className="min-w-0 flex-1">
        <div className="text-[12px] text-tui-fg">{label}</div>
        {hint && (
          <div className="mt-0.5 text-[11px] leading-snug text-tui-fg-muted">
            {hint}
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center">{control}</div>
    </div>
  );
}

/** Centered banner / call-out box for non-actionable explanations. */
function Callout({
  tone = "info",
  children,
}: {
  tone?: "info" | "warn";
  children: ReactNode;
}) {
  const cls =
    tone === "warn"
      ? "border-tui-warn/30 bg-[rgba(252,225,0,0.04)] text-tui-warn"
      : "border-tui-border bg-[var(--fluent-bg-subtle)] text-tui-fg-dim";
  return (
    <div className={`rounded-[6px] border px-3 py-2 text-[11px] ${cls}`}>
      {children}
    </div>
  );
}

// ─── general section ─────────────────────────────────────────────────

function GeneralSection() {
  const s = useSettingsStore();
  return (
    <>
      <SettingsGroup
        title="Startup"
        description="What zero does the first time you launch it on a fresh machine."
      >
        <Field
          label="Auto-install & start OVMS on launch"
          hint="Download OVMS if missing, then boot the default model so the first chat works immediately."
          control={
            <Toggle
              checked={s.auto_provision_ovms}
              onChange={(v) => void s.save({ auto_provision_ovms: v })}
              label="Auto-provision OVMS"
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup
        title="Agent behaviour"
        description="Controls how the agent plans, calls tools, and protects you from itself."
      >
        <Field
          label="Confirm destructive tools"
          hint="Prompt before any tool flagged as destructive (file deletes, shell writes…) runs."
          control={
            <Toggle
              checked={s.destructive_tool_confirm}
              onChange={(v) => void s.save({ destructive_tool_confirm: v })}
              label="Confirm destructive tools"
            />
          }
        />
        <Field
          label="Max iterations"
          hint="Upper bound on tool-call / response cycles per turn before the agent gives up."
          control={
            <TuiInput
              type="number"
              min={1}
              value={s.agent_max_iterations}
              onChange={(e) =>
                void s.save({
                  agent_max_iterations: Number(e.target.value) || 1,
                })
              }
              className="w-24"
            />
          }
        />
        <Field
          label="Lazy tool discovery"
          hint={
            <>
              Ship only the{" "}
              <code className="rounded bg-[var(--fluent-bg-subtle-pressed)] px-1 font-mono text-tui-fg">
                tools.list
              </code>{" "}
              built-in to the model on the first round of each turn. The model
              calls it on demand to learn what else is available; the rest of
              the catalogue is exposed automatically on the next round. Saves a
              lot of initial context when many MCP servers are wired up, at the
              cost of one extra round-trip on turns that need a tool.
            </>
          }
          control={
            <Toggle
              checked={s.lazy_tool_discovery}
              onChange={(v) => void s.save({ lazy_tool_discovery: v })}
              label="Lazy tool discovery"
            />
          }
        />
      </SettingsGroup>
    </>
  );
}

// ─── providers section ───────────────────────────────────────────────

function ProvidersSection() {
  const s = useSettingsStore();

  function updateProvider(id: string, patch: Partial<ProviderConfig>) {
    const next = s.providers.map((p) => (p.id === id ? { ...p, ...patch } : p));
    void s.save({ providers: next });
  }

  function addProvider() {
    const kind: ProviderKind = "ollama";
    const id = freshProviderId(s.providers);
    void s.save({
      providers: [
        ...s.providers,
        {
          id,
          kind,
          name: `${kind} (local)`,
          base_url: defaultBaseUrl(kind),
          enabled: true,
          sampling: { ...EMPTY_SAMPLING },
        },
      ],
    });
  }

  function deleteProvider(id: string) {
    const next = s.providers.filter((p) => p.id !== id);
    const patch: Partial<typeof s> = { providers: next };
    // If we just deleted the active one, fall back to whichever enabled
    // provider is still standing — keeps the chat runner pointed at
    // *something* instead of erroring out next turn.
    if (s.active_provider_id === id) {
      patch.active_provider_id = next.find((p) => p.enabled)?.id ?? null;
    }
    void s.save(patch);
  }

  return (
    <>
      <SettingsGroup
        title="Providers"
        description={`${s.providers.length} configured · choose one as active for the chat runner.`}
        action={
          <TuiButton size="sm" variant="default" onClick={addProvider}>
            + Add provider
          </TuiButton>
        }
      >
        {s.providers.length === 0 ? (
          <div className="py-6 text-center text-[11px] text-tui-fg-muted">
            No providers configured. Click <b>Add provider</b> to point zero at
            a local OVMS, Ollama, or vLLM endpoint.
          </div>
        ) : (
          <ul className="-mx-1 divide-y divide-tui-border">
            {s.providers.map((p) => (
              <ProviderCard
                key={p.id}
                provider={p}
                active={p.id === s.active_provider_id}
                onActivate={() => void s.save({ active_provider_id: p.id })}
                onChange={(patch) => updateProvider(p.id, patch)}
                onDelete={() => deleteProvider(p.id)}
              />
            ))}
          </ul>
        )}
      </SettingsGroup>

      <Callout>
        Base URL must point at an OpenAI-compatible endpoint (
        <code className="font-mono">/chat/completions</code>). OVMS, Ollama, and
        vLLM all expose one — the three kinds share the same provider
        implementation today; the field is mostly for documentation.
      </Callout>
    </>
  );
}

function ProviderCard({
  provider,
  active,
  onActivate,
  onChange,
  onDelete,
}: {
  provider: ProviderConfig;
  active: boolean;
  onActivate: () => void;
  onChange: (patch: Partial<ProviderConfig>) => void;
  onDelete: () => void;
}) {
  return (
    <li
      className={
        "rounded-[6px] px-2 py-2.5 transition-colors " +
        (active
          ? "bg-[var(--fluent-bg-subtle-selected)]"
          : "hover:bg-[var(--fluent-bg-subtle-hover)]")
      }
    >
      <div className="mb-2 flex items-center gap-2">
        <button
          onClick={onActivate}
          title={active ? "Active provider" : "Set as active"}
          aria-label={active ? "Active" : "Set active"}
          aria-pressed={active}
          className={
            "flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition-colors " +
            (active
              ? "border-tui-accent"
              : "border-[var(--fluent-stroke-control-strong)] hover:border-tui-fg")
          }
        >
          {active && <span className="h-2 w-2 rounded-full bg-tui-accent" />}
        </button>
        <KindBadge kind={provider.kind} />
        <TuiInput
          value={provider.name}
          onChange={(e) => onChange({ name: e.target.value })}
          placeholder="Provider name"
          className="!flex-1"
        />
        <label
          className="flex items-center gap-1.5 text-[11px] text-tui-fg-dim"
          title={provider.enabled ? "Enabled" : "Disabled"}
        >
          <Toggle
            checked={provider.enabled}
            onChange={(v) => onChange({ enabled: v })}
            label="Enabled"
          />
        </label>
        <button
          onClick={onDelete}
          className="flex h-7 w-7 items-center justify-center rounded text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-err"
          title="Delete provider"
          aria-label="Delete provider"
        >
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
        </button>
      </div>
      <div className="flex items-center gap-2 pl-6">
        <TuiSelect
          value={provider.kind}
          onChange={(e) => onChange({ kind: e.target.value as ProviderKind })}
          className="!w-24"
        >
          {PROVIDER_KINDS.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </TuiSelect>
        <TuiInput
          value={provider.base_url}
          onChange={(e) => onChange({ base_url: e.target.value })}
          placeholder={defaultBaseUrl(provider.kind)}
        />
      </div>
      <ProviderSampling
        value={provider.sampling ?? EMPTY_SAMPLING}
        onChange={(sampling) => onChange({ sampling })}
      />
    </li>
  );
}

/**
 * Collapsible "Sampling defaults" sub-card on each provider row.
 * Collapsed by default to keep the providers list scannable; the
 * count badge surfaces whether any field is currently overridden so
 * the user knows when this provider deviates from the model defaults.
 */
function ProviderSampling({
  value,
  onChange,
}: {
  value: SamplingConfig;
  onChange: (next: SamplingConfig) => void;
}) {
  const [open, setOpen] = useState(false);
  const overrideCount =
    (value.temperature != null ? 1 : 0) +
    (value.top_p != null ? 1 : 0) +
    (value.top_k != null ? 1 : 0);
  // Per-model profile defaults shown in the empty inputs so the user
  // sees what the runner will actually send when a field is left blank.
  // Provider configs are model-agnostic, so we use the currently loaded
  // OVMS model as the reference — the most common case is "only one
  // text-gen model loaded at a time", which makes this a faithful
  // preview. When nothing is loaded the helper falls back to the
  // generic profile (temperature 0.7, top_p/top_k upstream).
  const loadedModel = useServerStore((s) => s.info?.loaded_model ?? null);
  const defaults = modelSamplingDefaults(loadedModel);

  return (
    <div className="mt-2 pl-6">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={
          "flex w-full items-center gap-2 rounded-[4px] px-1.5 py-1 text-left text-[11px] " +
          "text-tui-fg-dim transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg"
        }
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
          style={{
            transform: open ? "rotate(90deg)" : "none",
            transition: "transform 150ms",
          }}
        >
          <path d="m3 2 3 3-3 3" />
        </svg>
        <span className="font-medium">Sampling defaults</span>
        <span className="text-[10.5px] text-tui-fg-muted">
          {overrideCount === 0
            ? "using model defaults"
            : `${overrideCount} override${overrideCount === 1 ? "" : "s"}`}
        </span>
      </button>
      {open && (
        <div className="mt-1 rounded-[6px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-3 py-2">
          <SamplingEditor
            value={value}
            onChange={onChange}
            temperaturePlaceholder={formatSamplingDefault(defaults.temperature)}
            topPPlaceholder={formatSamplingDefault(defaults.top_p)}
            topKPlaceholder={formatSamplingDefault(defaults.top_k)}
          />
          <p className="mt-2 border-t border-tui-border pt-1.5 text-[10.5px] text-tui-fg-muted">
            Applies to every chat that uses this provider. Individual chats can
            still override these from the chat header.
          </p>
        </div>
      )}
    </div>
  );
}

function KindBadge({ kind }: { kind: ProviderKind }) {
  return (
    <span
      className={
        "rounded-[4px] border border-tui-border bg-[var(--fluent-bg-subtle)] " +
        "px-1.5 py-px text-[10px] font-mono uppercase tracking-wide text-tui-fg-dim"
      }
    >
      {kind}
    </span>
  );
}

// ─── server section ──────────────────────────────────────────────────

function ServerSection() {
  const s = useSettingsStore();
  const serverStatus = useServerStore((st) => st.info?.status ?? null);
  const ovmsRunning = serverStatus === "running" || serverStatus === "starting";

  function patchOvms(patch: Partial<OvmsSettings>) {
    void s.save({ ovms: { ...s.ovms, ...patch } });
  }

  const usingCustomDevice = !DEVICE_CHOICES.includes(s.ovms.device);

  return (
    <>
      <SettingsGroup
        title="OpenVINO Model Server"
        description="Local inference server zero spawns and supervises."
      >
        <div className="py-2">
          <OvmsStatusCard />
        </div>
      </SettingsGroup>

      <InstallProgressCard />

      <LoadedModelsGroup />

      <SettingsGroup
        title="Runtime"
        description="Ports, device, and other knobs applied the next time OVMS (re)starts."
        action={
          ovmsRunning ? (
            <span className="text-[11px] text-tui-warn">restart to apply</span>
          ) : undefined
        }
      >
        <Field
          label="REST port"
          hint="HTTP endpoint OVMS serves /v3/* on."
          control={
            <TuiInput
              type="number"
              min={1}
              max={65535}
              value={s.ovms.rest_port}
              onChange={(e) =>
                patchOvms({ rest_port: Number(e.target.value) || 0 })
              }
              className="w-24"
            />
          }
        />
        <Field
          label="gRPC port"
          hint={
            <>
              0 disables OVMS's gRPC server. zero only uses REST, so the default
              is off to avoid colliding with other services on 9000.
            </>
          }
          control={
            <TuiInput
              type="number"
              min={0}
              max={65535}
              value={s.ovms.grpc_port}
              onChange={(e) =>
                patchOvms({ grpc_port: Number(e.target.value) || 0 })
              }
              className="w-24"
            />
          }
        />
        <Field
          label="Device"
          hint="OpenVINO target device. AUTO lets the runtime pick; MULTI/HETERO compose multiple."
          control={
            <TuiSelect
              value={usingCustomDevice ? "custom" : s.ovms.device}
              onChange={(e) => {
                const v = e.target.value;
                if (v === "custom") return;
                const seed = COMPOUND_DEVICE_SEEDS[v];
                patchOvms({ device: seed ?? v });
              }}
              className="!w-36"
            >
              {DEVICE_CHOICES.map((d) => (
                <option key={d} value={d}>
                  {COMPOUND_DEVICE_SEEDS[d] ? `${d}…` : d}
                </option>
              ))}
              <option value="custom">Custom…</option>
            </TuiSelect>
          }
        />
        <Field
          label="Custom device string"
          hint="Pass-through OpenVINO device descriptor — e.g. MULTI:GPU,CPU."
          control={
            <TuiInput
              value={s.ovms.device}
              onChange={(e) => patchOvms({ device: e.target.value })}
              placeholder="e.g. MULTI:GPU,CPU"
              className="w-56"
            />
          }
        />
      </SettingsGroup>

      <AdvancedOvms ovms={s.ovms} patch={patchOvms} />
    </>
  );
}

// ─── loaded models inspector ─────────────────────────────────────────

/**
 * Per-model card list backed by OVMS's TFS REST surface
 * (`GET /v1/config` for aggregated state, `GET /v1/models/<id>/metadata`
 * for tensor signatures). Lets the user inspect what OVMS is actually
 * serving without leaving the app or hitting curl by hand.
 *
 * State strategy:
 *   - The model *list* comes from `useServerStore().info.loaded_models`
 *     (already kept fresh by the `ovms://status` event listener).
 *   - The TFS state overlay is fetched on mount and on a refresh click.
 *     We don't subscribe via WS because OVMS doesn't expose one; the
 *     refresh affordance is the explicit way to re-poll.
 *   - Per-model metadata is fetched lazily on first expand and cached
 *     in component state so re-opening the same row is free.
 */
function LoadedModelsGroup() {
  const loadedModels = useServerStore((s) => s.info?.loaded_models) ?? null;
  const status = useServerStore((s) => s.info?.status ?? null);
  const getConfigStatus = useServerStore((s) => s.getConfigStatus);
  const removeModel = useServerStore((s) => s.removeModel);

  const running = status === "running";
  const [tfs, setTfs] = useState<TfsConfigStatus | null>(null);
  const [tfsLoading, setTfsLoading] = useState(false);
  const [tfsError, setTfsError] = useState<string | null>(null);

  async function refreshTfs() {
    if (!running) return;
    setTfsLoading(true);
    setTfsError(null);
    try {
      const next = await getConfigStatus();
      setTfs(next);
    } catch (e) {
      setTfsError(String(e));
    } finally {
      setTfsLoading(false);
    }
  }

  // Initial fetch + re-fetch whenever the loaded-models lineup changes
  // (e.g. user just swapped models from the chat header). Keying on the
  // joined id list avoids the deeper-equality dance — the snapshot is
  // small and stable.
  const modelKey = (loadedModels ?? []).map((m) => m.id).join("|");
  useEffect(() => {
    if (!running) {
      setTfs(null);
      return;
    }
    void refreshTfs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [running, modelKey]);

  const models = loadedModels ?? [];

  return (
    <SettingsGroup
      title="Loaded models"
      description={
        running
          ? `${models.length} model${models.length === 1 ? "" : "s"} live on OVMS. Inspect tensor metadata or drop a single model without restarting.`
          : "OVMS is not running — start it to inspect served models."
      }
      action={
        running ? (
          <TuiButton
            size="sm"
            variant="subtle"
            onClick={() => void refreshTfs()}
            disabled={tfsLoading}
            title="Re-poll GET /v1/config"
          >
            {tfsLoading ? (
              <>
                <Spinner size="sm" /> Refresh
              </>
            ) : (
              "Refresh"
            )}
          </TuiButton>
        ) : undefined
      }
    >
      {!running ? (
        <div className="py-6 text-center text-[11px] text-tui-fg-muted">
          Start OVMS from the panel above to inspect its loaded models.
        </div>
      ) : models.length === 0 ? (
        <div className="py-6 text-center text-[11px] text-tui-fg-muted">
          No models loaded.
        </div>
      ) : (
        <ul className="divide-y divide-tui-border">
          {models.map((m) => (
            <LoadedModelRow
              key={m.id}
              model={m}
              tfs={tfs?.[m.id]}
              onRemove={() => void removeModel(m.id)}
            />
          ))}
        </ul>
      )}
      {tfsError && (
        <div className="mt-2 rounded-[6px] border border-tui-err/30 bg-[rgba(255,153,164,0.06)] px-3 py-2 text-[11px] text-tui-err">
          {tfsError}
        </div>
      )}
    </SettingsGroup>
  );
}

function LoadedModelRow({
  model,
  tfs,
  onRemove,
}: {
  model: LoadedModel;
  tfs?: TfsConfigStatus[string];
  onRemove: () => void;
}) {
  // TFS reports per-version state. Most zero models ship a single
  // version (`1`) but the wire shape is an array, so collapse it into
  // a one-line summary keyed off the worst-case state.
  const stateSummary = summariseTfsState(tfs);

  return (
    <li className="py-2">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span
              className="truncate font-mono text-[12px] text-tui-fg"
              title={model.id}
            >
              {model.id}
            </span>
            <TaskBadge task={model.task} />
            {stateSummary && (
              <StateBadge tone={stateSummary.tone} label={stateSummary.label} />
            )}
          </div>
          {(model.tool_parser || stateSummary?.detail) && (
            <div className="mt-0.5 truncate text-[11px] text-tui-fg-muted">
              {model.tool_parser && (
                <>
                  tool_parser:{" "}
                  <code className="font-mono">{model.tool_parser}</code>
                </>
              )}
              {model.tool_parser && stateSummary?.detail && " · "}
              {stateSummary?.detail}
            </div>
          )}
        </div>
        <button
          onClick={onRemove}
          className="rounded-[4px] px-2 py-1 text-[11px] text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-err"
          title="Remove from live serving set"
        >
          Unload
        </button>
      </div>
    </li>
  );
}

function TaskBadge({ task }: { task: string }) {
  return (
    <span className="shrink-0 rounded-[4px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-1.5 py-px text-[10px] uppercase tracking-wide text-tui-fg-dim">
      {task.replace(/_/g, " ")}
    </span>
  );
}

function StateBadge({
  tone,
  label,
}: {
  tone: "ok" | "warn" | "err" | "muted";
  label: string;
}) {
  const cls = {
    ok: "text-tui-ok border-tui-ok/30 bg-[rgba(108,203,95,0.08)]",
    warn: "text-tui-warn border-tui-warn/30 bg-[rgba(252,225,0,0.06)]",
    err: "text-tui-err border-tui-err/30 bg-[rgba(255,153,164,0.08)]",
    muted: "text-tui-fg-muted border-tui-border bg-[var(--fluent-bg-subtle)]",
  }[tone];
  return (
    <span
      className={`shrink-0 rounded-[4px] border px-1.5 py-px text-[10px] font-medium uppercase tracking-wide ${cls}`}
    >
      {label}
    </span>
  );
}

/** Collapse a `ModelStatus` (array of versions) into a single chip-friendly
 *  summary. Anything that isn't `AVAILABLE` is surfaced as warn/err. */
function summariseTfsState(s: TfsConfigStatus[string] | undefined): {
  tone: "ok" | "warn" | "err" | "muted";
  label: string;
  detail?: string;
} | null {
  if (!s) return null;
  const versions = s.model_version_status ?? [];
  if (versions.length === 0) {
    return { tone: "muted", label: "unknown" };
  }
  // Pick the most concerning state for the badge — errors trump
  // loading, loading trumps available.
  const order = ["END", "UNAVAILABLE", "LOADING", "AVAILABLE", "START"];
  const worst = versions.reduce((acc, v) =>
    order.indexOf(v.state) < order.indexOf(acc.state) ? v : acc,
  );
  const tone =
    worst.state === "AVAILABLE"
      ? "ok"
      : worst.state === "LOADING" || worst.state === "START"
        ? "warn"
        : "err";
  const versionsLabel =
    versions.length === 1
      ? `v${versions[0].version}`
      : `${versions.length} versions`;
  const detail =
    worst.status?.error_code && worst.status.error_code !== "OK"
      ? `${worst.status.error_code}: ${worst.status.error_message}`
      : versionsLabel;
  return { tone, label: worst.state.toLowerCase(), detail };
}

// ─── system section ──────────────────────────────────────────────────

function SystemSection() {
  return (
    <SettingsGroup
      title="Host"
      description="Hardware zero can see on this machine. Read-only — useful for picking a device or filing bug reports."
    >
      <div className="py-2">
        <SystemSpecsCard />
      </div>
    </SettingsGroup>
  );
}

// ─── advanced OVMS options ──────────────────────────────────────────

const LOG_LEVELS = ["DEBUG", "INFO", "ERROR"] as const;

/**
 * Collapsed by default — advanced ovms.exe CLI flags that most users
 * don't need to touch. Each field maps 1:1 to a flag documented at
 * https://docs.openvino.ai/2026/model-server/ovms_docs_parameters.html
 * and is forwarded to the spawned process by `OvmsSettings::cli_args`
 * on the Rust side.
 */
function AdvancedOvms({
  ovms,
  patch,
}: {
  ovms: OvmsSettings;
  patch: (p: Partial<OvmsSettings>) => Promise<void> | void;
}) {
  // Local mirror so the textarea stays editable while the user is mid-typing
  // (each keystroke triggers an IPC save otherwise, and the value coming
  // back from the store would be re-stringified and clobber the cursor).
  const [extraDraft, setExtraDraft] = useState<string>(
    ovms.extra_args.join("\n"),
  );

  /** Split the textarea on whitespace so `--flag value` lines yield two argv
   * tokens. Blank lines / runs of whitespace are dropped. */
  function commitExtraArgs(text: string) {
    setExtraDraft(text);
    const tokens = text
      .split(/\s+/)
      .map((t) => t.trim())
      .filter((t) => t.length > 0);
    void patch({ extra_args: tokens });
  }

  /** Treat empty/whitespace as `null` so we round-trip cleanly to the Rust
   * `Option<String>` and never push `--flag ""` to ovms.exe. */
  function patchOpt<K extends keyof OvmsSettings>(key: K, raw: string) {
    const v = raw.trim();
    void patch({ [key]: v === "" ? null : raw } as Partial<OvmsSettings>);
  }

  return (
    <details className="fluent-mica group overflow-hidden rounded-[10px] border border-tui-border shadow-[var(--fluent-shadow-2)]">
      <summary
        className={
          "flex cursor-pointer select-none items-center justify-between gap-3 px-4 py-2.5 " +
          "text-[12px] font-semibold text-tui-fg-dim transition-colors hover:text-tui-fg " +
          "[&::-webkit-details-marker]:hidden"
        }
      >
        <span className="flex items-center gap-2">
          <ChevronIcon />
          Advanced OVMS options
        </span>
        <span className="text-[11px] font-normal text-tui-fg-muted">
          ovms.exe CLI flags
        </span>
      </summary>
      <div className="border-t border-tui-border px-4 py-1.5">
        <Field
          label="Log level"
          control={
            <TuiSelect
              value={ovms.log_level ?? ""}
              onChange={(e) =>
                void patch({ log_level: e.target.value || null })
              }
              className="!w-36"
            >
              <option value="">(default · INFO)</option>
              {LOG_LEVELS.map((lv) => (
                <option key={lv} value={lv}>
                  {lv}
                </option>
              ))}
            </TuiSelect>
          }
        />
        <Field
          label="Log path"
          hint="Where ovms.exe writes its log. Defaults to stderr only."
          control={
            <TuiInput
              value={ovms.log_path ?? ""}
              onChange={(e) => patchOpt("log_path", e.target.value)}
              placeholder="(stderr only)"
              className="w-64"
            />
          }
        />
        <Field
          label="Cache dir"
          hint="Compiled-model cache. Massively speeds up subsequent starts."
          control={
            <TuiInput
              value={ovms.cache_dir ?? ""}
              onChange={(e) => patchOpt("cache_dir", e.target.value)}
              placeholder="speeds up subsequent starts"
              className="w-64"
            />
          }
        />
        <Field
          label="API key file"
          hint="First line of this file is the bearer token required on /v3/*."
          control={
            <TuiInput
              value={ovms.api_key_file ?? ""}
              onChange={(e) => patchOpt("api_key_file", e.target.value)}
              placeholder="(no auth)"
              className="w-64"
            />
          }
        />
        <Field
          label="CORS allowed origins"
          hint="Sets Access-Control-Allow-Origin. Use * for any."
          control={
            <TuiInput
              value={ovms.allowed_origins ?? ""}
              onChange={(e) => patchOpt("allowed_origins", e.target.value)}
              placeholder="* (any)"
              className="w-64"
            />
          }
        />
        <Field
          label="Extra args"
          align="start"
          hint={
            <>
              Forwarded verbatim to ovms.exe — one token per line or
              space-separated. See{" "}
              <code className="font-mono">ovms_docs_parameters.html</code> for
              the full flag list.
            </>
          }
          control={
            <TuiTextarea
              rows={4}
              value={extraDraft}
              onChange={(e) => commitExtraArgs(e.target.value)}
              placeholder={`--grpc_workers\n4\n--allowed_media_domains\nall`}
              className="w-64 font-mono"
            />
          }
        />
      </div>
    </details>
  );
}

// ─── icons (inline SVG — match the sidebar's stroke weight) ─────────

function SlidersIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
    >
      <path d="M4 6h10M18 6h2M4 12h6M14 12h6M4 18h12M20 18h0" />
      <circle cx="16" cy="6" r="2" />
      <circle cx="12" cy="12" r="2" />
      <circle cx="18" cy="18" r="2" />
    </svg>
  );
}

function PlugIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M9 2v4M15 2v4" />
      <path d="M7 6h10v5a5 5 0 0 1-10 0V6Z" />
      <path d="M12 16v6" />
    </svg>
  );
}

function ServerIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="3" y="4" width="18" height="7" rx="1.5" />
      <rect x="3" y="13" width="18" height="7" rx="1.5" />
      <path d="M7 7.5h.01M7 16.5h.01" />
    </svg>
  );
}

function ChipIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="6" y="6" width="12" height="12" rx="1.5" />
      <path d="M9 9h6v6H9z" />
      <path d="M3 9h3M3 12h3M3 15h3M18 9h3M18 12h3M18 15h3M9 3v3M12 3v3M15 3v3M9 18v3M12 18v3M15 18v3" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="transition-transform duration-150 group-open:rotate-90"
    >
      <path d="m3 2 3 3-3 3" />
    </svg>
  );
}
