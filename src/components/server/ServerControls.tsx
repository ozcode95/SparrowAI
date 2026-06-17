import { TuiButton } from "@/components/tui/Button";
import { ProgressBar } from "@/components/tui/ProgressBar";
import { Spinner } from "@/components/tui/Spinner";
import { useServerStore } from "@/stores/server";
import { useSettingsStore } from "@/stores/settings";
import { useSystemStore } from "@/stores/system";
import { bytes } from "@/lib/format";
import type { ReactNode } from "react";

/**
 * OVMS lifecycle + host-system building blocks used by the Settings
 * page. They used to live behind a single `ServerControls` umbrella
 * component that rendered three back-to-back panels; the redesigned
 * Settings places each block in the appropriate section instead, so
 * they're exported as separate pieces here.
 */

// ─── status tone helpers ─────────────────────────────────────────────

type StatusTone = "ok" | "warn" | "err" | "muted";

function toneClass(t: StatusTone): string {
  return {
    ok: "bg-tui-ok",
    warn: "bg-tui-warn",
    err: "bg-tui-err",
    muted: "bg-tui-fg-muted",
  }[t];
}

function statusTone(status: string): StatusTone {
  switch (status) {
    case "running":
      return "ok";
    case "starting":
    case "installing":
    case "stopping":
      return "warn";
    case "error":
      return "err";
    default:
      return "muted";
  }
}

/** Colored dot + status label, used in the OVMS status hero. */
export function StatusDot({
  tone,
  label,
  className = "",
}: {
  tone: StatusTone;
  label?: ReactNode;
  className?: string;
}) {
  return (
    <span className={`inline-flex items-center gap-1.5 ${className}`}>
      <span
        className={`inline-block h-2 w-2 rounded-full ${toneClass(tone)}`}
      />
      {label != null && (
        <span className="text-[11px] text-tui-fg-dim">{label}</span>
      )}
    </span>
  );
}

// ─── OVMS lifecycle hero ─────────────────────────────────────────────

/**
 * Full OVMS status surface: prominent status pill, version/runtime
 * facts laid out as a key/value grid, and the install / update /
 * start / stop button row. The tool-parser warning (when a loaded
 * model has no `tool_parser` configured) is kept here too so the
 * caveat travels with the loaded-model line.
 */
export function OvmsStatusCard() {
  const info = useServerStore((s) => s.info);
  const install = useServerStore((s) => s.install);
  const update = useServerStore((s) => s.update);
  const start = useServerStore((s) => s.start);
  const stop = useServerStore((s) => s.stop);
  const ovmsCfg = useSettingsStore((s) => s.ovms);
  const defaultModel = useSettingsStore((s) => s.default_model);

  const status = info?.status ?? "unknown";
  const running = status === "running";
  const installing = status === "installing";
  const installed = status !== "not_installed" && status !== "installing";
  const hasUpdate =
    info?.latest_version != null &&
    info?.installed_version != null &&
    info.latest_version !== info.installed_version;
  const startModel = defaultModel ?? info?.loaded_model ?? "";

  return (
    <div className="space-y-3">
      {/* Status hero */}
      <div className="flex items-center justify-between gap-3 rounded-[8px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-3 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <StatusDot tone={statusTone(status)} />
          <div className="min-w-0">
            <div className="text-[13px] font-semibold capitalize text-tui-fg">
              {status.replace("_", " ")}
            </div>
            <div className="truncate text-[11px] text-tui-fg-muted">
              {info?.base_url ?? "—"}
            </div>
          </div>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-1.5">
          {!installed && !installing && (
            <TuiButton variant="primary" onClick={() => void install()}>
              Install
            </TuiButton>
          )}
          {installing && (
            <TuiButton disabled>
              <Spinner size="sm" /> Installing…
            </TuiButton>
          )}
          {installed && (
            <TuiButton
              variant={hasUpdate ? "primary" : "default"}
              size="sm"
              onClick={() => void update()}
            >
              {hasUpdate ? "Update available" : "Check for updates"}
            </TuiButton>
          )}
          {!running && installed && (
            <TuiButton
              variant="primary"
              size="sm"
              disabled={!startModel}
              onClick={() => void start(startModel)}
              title={
                startModel
                  ? `Start OVMS with ${startModel}`
                  : "Pick a default model on the Models page first"
              }
            >
              Start
            </TuiButton>
          )}
          {running && (
            <TuiButton variant="danger" size="sm" onClick={() => void stop()}>
              Stop
            </TuiButton>
          )}
        </div>
      </div>

      {/* Facts grid */}
      <dl className="grid grid-cols-[7.5rem_1fr] gap-x-3 gap-y-1.5 text-[11px]">
        <Fact label="Installed" value={info?.installed_version ?? "—"} />
        <Fact
          label="Latest"
          value={
            info?.latest_version ? (
              <span className={hasUpdate ? "text-tui-warn" : "text-tui-fg"}>
                {info.latest_version}
                {hasUpdate && " ↑"}
              </span>
            ) : (
              "—"
            )
          }
        />
        <Fact label="PID" value={info?.pid ?? "—"} />
        <Fact
          label="Ports"
          value={`${ovmsCfg.rest_port} REST · ${
            ovmsCfg.grpc_port === 0
              ? "gRPC disabled"
              : `${ovmsCfg.grpc_port} gRPC`
          }`}
        />
        <Fact label="Device" value={ovmsCfg.device || "—"} />
        <Fact label="Loaded" value={info?.loaded_model ?? "—"} />
        <Fact
          label="Tool parser"
          value={
            info?.loaded_model ? (
              info.loaded_model_tool_parser ? (
                info.loaded_model_tool_parser
              ) : (
                <span
                  className="text-tui-warn"
                  title="No tool_parser configured for this model — native function calling is disabled."
                >
                  none
                </span>
              )
            ) : (
              "—"
            )
          }
        />
      </dl>

      {info?.last_error && (
        <div className="rounded-[6px] border border-tui-err/30 bg-[rgba(255,153,164,0.06)] px-3 py-2 text-[11px] text-tui-err">
          {info.last_error}
        </div>
      )}
      {info?.loaded_model && !info.loaded_model_tool_parser && (
        <div className="rounded-[6px] border border-tui-warn/30 bg-[rgba(252,225,0,0.04)] px-3 py-2 text-[11px] text-tui-warn">
          No <code className="font-mono">tool_parser</code> configured — native
          function calling is disabled for this model. Tool requests fall back
          to a text protocol; success depends on the model.
        </div>
      )}
    </div>
  );
}

function Fact({ label, value }: { label: string; value: ReactNode }) {
  return (
    <>
      <dt className="text-tui-fg-muted">{label}</dt>
      <dd className="min-w-0 truncate text-tui-fg">{value}</dd>
    </>
  );
}

// ─── install progress ────────────────────────────────────────────────

/** Live download/extract progress shown while an install is running. */
export function InstallProgressCard() {
  const progress = useServerStore((s) => s.installProgress);
  if (!progress) return null;
  return (
    <div className="space-y-1.5 rounded-[8px] border border-tui-border bg-[var(--fluent-bg-subtle)] px-3 py-2.5 text-[11px]">
      <div className="flex items-center justify-between">
        <span className="font-semibold text-tui-fg">Installing OVMS</span>
        <span className="text-tui-fg-muted">
          {progress.stage.replace("_", " ")}
        </span>
      </div>
      <div className="truncate text-tui-fg-dim">{progress.message}</div>
      <ProgressBar
        value={progress.percent}
        max={1}
        label={
          progress.bytes_total
            ? `${bytes(progress.bytes_done)} / ${bytes(progress.bytes_total)}`
            : `${Math.round(progress.percent * 100)}%`
        }
        tone={
          progress.stage === "error"
            ? "err"
            : progress.stage === "done"
              ? "ok"
              : "default"
        }
      />
    </div>
  );
}

// ─── host system specs ───────────────────────────────────────────────

/** Read-only hardware snapshot for the System settings section. */
export function SystemSpecsCard() {
  const specs = useSystemStore((s) => s.specs);

  if (!specs) {
    return (
      <span className="inline-flex items-center gap-2 text-[12px] text-tui-fg-muted">
        <Spinner size="sm" /> Probing host…
      </span>
    );
  }

  return (
    <dl className="grid grid-cols-[7.5rem_1fr] gap-x-3 gap-y-1.5 text-[12px]">
      <Fact label="OS" value={`${specs.os} ${specs.os_version}`} />
      <Fact label="Architecture" value={specs.arch} />
      <Fact label="CPU" value={specs.cpu_brand} />
      <Fact
        label="Cores"
        value={`${specs.cpu_physical_cores} physical · ${specs.cpu_logical_cores} logical`}
      />
      <Fact label="RAM" value={bytes(specs.ram_total_mb * 1024 * 1024)} />
      {specs.gpus.map((g, i) => (
        <Fact
          key={`gpu-${i}`}
          label={`GPU ${i}`}
          value={`${g.name} (${g.kind})`}
        />
      ))}
      {specs.npus.map((n, i) => (
        <Fact key={`npu-${i}`} label={`NPU ${i}`} value={n.name} />
      ))}
    </dl>
  );
}
