import { forwardRef, useEffect, useMemo, useRef, useState } from "react";
import { TuiButton } from "@/components/tui/Button";
import { ProgressBar } from "@/components/tui/ProgressBar";
import { Spinner } from "@/components/tui/Spinner";
import { ParamsModal } from "@/components/models/ParamsModal";
import {
  useModelsStore,
  type DownloadProgress,
  type HfModelSummary,
  type LocalModel,
} from "@/stores/models";
import {
  useServerStore,
  type LoadedModel,
  type TfsModelStatus,
} from "@/stores/server";
import { useSystemStore } from "@/stores/system";
import { bytes, relativeTime } from "@/lib/format";
import type { ModelDeployParams, ModelTaskKind } from "@/lib/modelParams";
import {
  pipelineTagToTaskKind,
  CATEGORY_META,
  CATEGORY_ORDER,
} from "@/lib/modelCategory";
import {
  RECOMMENDED_BY_KIND,
  type RecommendedModel,
  type RecommendationMap,
} from "@/lib/recommendedModels";

/** Pop-over only ever shows the top N HF hits. */
const RESULT_LIMIT = 5;
/** Auto-search debounce — fires N ms after the user stops typing. */
const SEARCH_DEBOUNCE_MS = 250;

/**
 * Models page — two‑view layout:
 *
 *   ┌─────────────────────────────────────────────┐
 *   │         [ search OpenVINO IR … ]            │  ← sticky search
 *   │              ▼ popover (top 5)              │
 *   ├─────────────────────────────────────────────┤
 *   │     [ Your Library ]   [ Recommended ]      │  ← view toggle
 *   ├─────────────────────────────────────────────┤
 *   │  ── Text Generation · 1 loaded · 3 local ── │
 *   │  ┌──────────┐ ┌──────────┐                  │
 *   │  │ card     │ │ card     │                  │
 *   │  └──────────┘ └──────────┘                  │
 *   │                                             │
 *   │  ── Embeddings · 0 loaded · 1 local ─────── │
 *   │  ┌──────────┐                               │
 *   │  │ card     │                               │
 *   │  └──────────┘                               │
 *   │                                             │
 *   │  (empty categories hidden in Library view)  │
 *   └─────────────────────────────────────────────┘
 *
 * Library view: groups `local` models by inferred `ModelTaskKind`;
 * empty categories hidden. Recommended view: always shows every
 * category with curated model cards.
 */
export function ModelsView() {
  // ── search state ─────────────────────────────────────────────────
  const query = useModelsStore((s) => s.query);
  const setQuery = useModelsStore((s) => s.setQuery);
  const searching = useModelsStore((s) => s.searching);
  const results = useModelsStore((s) => s.results);
  const search = useModelsStore((s) => s.search);

  // ── local library ────────────────────────────────────────────────
  const local = useModelsStore((s) => s.local);
  const downloads = useModelsStore((s) => s.downloads);
  const refreshLocal = useModelsStore((s) => s.refreshLocal);
  const download = useModelsStore((s) => s.download);
  const cancel = useModelsStore((s) => s.cancel);
  const remove = useModelsStore((s) => s.remove);
  const update = useModelsStore((s) => s.update);
  const getParams = useModelsStore((s) => s.getParams);
  const suggestParams = useModelsStore((s) => s.suggestParams);
  const setStoredParams = useModelsStore((s) => s.setParams);

  // ── OVMS lifecycle ───────────────────────────────────────────────
  const loadedModels = useServerStore((s) => s.info?.loaded_models);
  const loadedModelIds = useMemo(
    () => new Set((loadedModels ?? []).map((m) => m.id)),
    [loadedModels],
  );
  const loadModel = useServerStore((s) => s.loadModel);
  const removeModel = useServerStore((s) => s.removeModel);

  // ── remove errors (per‑card) ─────────────────────────────────────
  const [removeErrors, setRemoveErrors] = useState<Record<string, string>>({});
  function dismissRemoveError(id: string) {
    setRemoveErrors((prev) => {
      if (!(id in prev)) return prev;
      const { [id]: _drop, ...rest } = prev;
      return rest;
    });
  }
  async function handleRemove(id: string) {
    dismissRemoveError(id);
    try {
      await remove(id);
    } catch (e) {
      setRemoveErrors((prev) => ({
        ...prev,
        [id]: e instanceof Error ? e.message : String(e),
      }));
    }
  }

  // ── load / unload state (per‑card) ───────────────────────────────
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({});
  const [pendingLoads, setPendingLoads] = useState<Set<string>>(new Set());
  function dismissLoadError(id: string) {
    setLoadErrors((prev) => {
      if (!(id in prev)) return prev;
      const { [id]: _drop, ...rest } = prev;
      return rest;
    });
  }
  async function handleLoad(id: string) {
    dismissLoadError(id);
    setPendingLoads((prev) => {
      const next = new Set(prev);
      next.add(id);
      return next;
    });
    try {
      await loadModel(id);
    } catch (e) {
      setLoadErrors((prev) => ({
        ...prev,
        [id]: e instanceof Error ? e.message : String(e),
      }));
    } finally {
      setPendingLoads((prev) => {
        if (!prev.has(id)) return prev;
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }
  async function handleUnload(id: string) {
    dismissLoadError(id);
    setPendingLoads((prev) => {
      const next = new Set(prev);
      next.add(id);
      return next;
    });
    try {
      await removeModel(id);
    } catch (e) {
      setLoadErrors((prev) => ({
        ...prev,
        [id]: e instanceof Error ? e.message : String(e),
      }));
    } finally {
      setPendingLoads((prev) => {
        if (!prev.has(id)) return prev;
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }

  // ── ParamsModal target ───────────────────────────────────────────
  const [paramsTarget, setParamsTarget] = useState<
    | { id: string; mode: "installed" }
    | { id: string; mode: "predownload" }
    | null
  >(null);

  // ── view toggle + search popover ─────────────────────────────────
  const [view, setView] = useState<"library" | "recommended">("library");
  const [popoverOpen, setPopoverOpen] = useState(false);
  const searchWrapRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Initial load
  useEffect(() => {
    void refreshLocal();
    void search();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Debounced auto‑search
  useEffect(() => {
    const id = window.setTimeout(() => void search(), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  // Close popover on outside click / Escape
  useEffect(() => {
    if (!popoverOpen) return;
    function onPointer(e: MouseEvent) {
      if (searchWrapRef.current?.contains(e.target as Node)) return;
      setPopoverOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setPopoverOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [popoverOpen]);

  // ── Derive category groupings ────────────────────────────────────

  /** Local models grouped by inferred task kind. */
  const modelsByKind = useMemo(() => {
    const map: Record<ModelTaskKind, LocalModel[]> = {
      text_generation: [],
      embeddings: [],
      rerank: [],
      image_generation: [],
      text2speech: [],
      speech2text: [],
    };
    for (const m of local) {
      const kind = pipelineTagToTaskKind(m.pipeline_tag) ?? "text_generation";
      if (kind in map) map[kind].push(m);
    }
    return map;
  }, [local]);

  /** OVMS‑loaded model ids keyed by task kind. */
  const loadedByKind = useMemo(() => {
    const map: Record<ModelTaskKind, LoadedModel[]> = {
      text_generation: [],
      embeddings: [],
      rerank: [],
      image_generation: [],
      text2speech: [],
      speech2text: [],
    };
    for (const lm of loadedModels ?? []) {
      const kind = lm.task as ModelTaskKind;
      if (kind in map) map[kind].push(lm);
    }
    return map;
  }, [loadedModels]);

  /** Local HF ids (for deduping downloads). */
  const localHfIds = useMemo(
    () => new Set(local.map((l) => l.hf_id).filter((x): x is string => !!x)),
    [local],
  );

  // ── Active downloads (cards before local manifest catches up) ────
  const activeDownloads = useMemo(() => {
    return Object.values(downloads).filter(
      (d) =>
        (d.state === "pending" ||
          d.state === "downloading" ||
          d.state === "verifying") &&
        !localHfIds.has(d.model_id),
    );
  }, [downloads, localHfIds]);

  const topResults = results.slice(0, RESULT_LIMIT);
  const showPopover = popoverOpen && (searching || topResults.length > 0);

  async function handleConfiguredDownload(
    id: string,
    params: ModelDeployParams,
  ) {
    setParamsTarget(null);
    setPopoverOpen(false);
    await download(id, params);
  }

  async function startDownload(id: string) {
    setPopoverOpen(false);
    inputRef.current?.blur();
    await download(id);
  }

  /** One‑click "smart download" for recommended models: auto‑detect
   *  task config then kick off the download without a modal. */
  async function handleSmartDownload(hfId: string) {
    setPopoverOpen(false);
    inputRef.current?.blur();
    try {
      const params = await suggestParams(hfId);
      await download(hfId, params);
    } catch {
      // suggestion failed — fall back to default download
      await download(hfId);
    }
  }

  // ── Render ───────────────────────────────────────────────────────
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {/* ── Search bar (centered, sticky‑ish) ──────────────────────── */}
      <div className="mx-auto mt-6 w-full max-w-[1200px] px-4">
        <div ref={searchWrapRef} className="relative">
          <SearchBox
            ref={inputRef}
            value={query}
            onChange={(v) => {
              setQuery(v);
              setPopoverOpen(true);
            }}
            onFocus={() => setPopoverOpen(true)}
            searching={searching}
            placeholder="Search OpenVINO models…"
          />
          {showPopover && (
            <SearchResultsPopover
              items={topResults}
              searching={searching}
              localIds={localHfIds}
              downloads={downloads}
              onDownload={(id) => void startDownload(id)}
              onConfigure={(id) => setParamsTarget({ id, mode: "predownload" })}
              onUpdate={(id) => void update(id)}
              onCancel={(id) => void cancel(id)}
            />
          )}
        </div>
      </div>

      {/* ── View toggle ───────────────────────────────────────────── */}
      <div className="flex shrink-0 justify-center pt-5 pb-1">
        <ViewTabs value={view} onChange={setView} />
      </div>

      {/* ── Category sections (scrollable) ─────────────────────────── */}
      <div className="mx-auto flex min-h-0 w-full max-w-[1200px] flex-1 flex-col gap-6 overflow-auto px-4 pb-8 pt-2">
        {view === "library" ? (
          <LibraryView
            modelsByKind={modelsByKind}
            loadedModelIds={loadedModelIds}
            loadedByKind={loadedByKind}
            localHfIds={localHfIds}
            activeDownloads={activeDownloads}
            pendingLoads={pendingLoads}
            removeErrors={removeErrors}
            loadErrors={loadErrors}
            onLoad={(id) => void handleLoad(id)}
            onUnload={(id) => void handleUnload(id)}
            onRemove={(id) => void handleRemove(id)}
            onDismissRemoveError={dismissRemoveError}
            onDismissLoadError={dismissLoadError}
            onParams={(id, hfId) =>
              setParamsTarget({ id: hfId ?? id, mode: "installed" })
            }
            onCancel={cancel}
            onSwitchToRecommended={() => setView("recommended")}
          />
        ) : (
          <RecommendedView
            localHfIds={localHfIds}
            loadedModelIds={loadedModelIds}
            downloads={downloads}
            onSmartDownload={(id) => void handleSmartDownload(id)}
            onConfigure={(id) => setParamsTarget({ id, mode: "predownload" })}
            onCancel={cancel}
          />
        )}
      </div>

      {paramsTarget && (
        <ParamsModal
          modelId={paramsTarget.id}
          load={paramsTarget.mode === "installed" ? getParams : suggestParams}
          save={
            paramsTarget.mode === "installed"
              ? setStoredParams
              : handleConfiguredDownload
          }
          subtitle={
            paramsTarget.mode === "predownload"
              ? "task, pipeline type, and tool parser were auto‑detected from HuggingFace metadata — review before downloading"
              : undefined
          }
          onClose={() => setParamsTarget(null)}
        />
      )}
    </div>
  );
}

// ─── view toggle ────────────────────────────────────────────────────

function ViewTabs({
  value,
  onChange,
}: {
  value: "library" | "recommended";
  onChange: (v: "library" | "recommended") => void;
}) {
  return (
    <div
      role="tablist"
      aria-label="Model view"
      className={
        "inline-flex items-center gap-0.5 rounded-[6px] " +
        "border border-tui-border bg-[var(--fluent-bg-subtle)] p-0.5"
      }
    >
      {(["library", "recommended"] as const).map((tab) => {
        const active = tab === value;
        return (
          <button
            key={tab}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(tab)}
            className={
              "rounded-[4px] px-3.5 py-1 text-[12px] font-medium transition-colors " +
              (active
                ? "bg-[var(--fluent-bg-subtle-selected)] font-semibold text-tui-accent shadow-[var(--fluent-shadow-2)]"
                : "text-tui-fg-muted hover:text-tui-fg hover:bg-[var(--fluent-bg-subtle-hover)]")
            }
          >
            {tab === "library" ? "Your Library" : "Recommended"}
          </button>
        );
      })}
    </div>
  );
}

// ─── library view ───────────────────────────────────────────────────

function LibraryView({
  modelsByKind,
  loadedModelIds,
  loadedByKind,
  localHfIds,
  activeDownloads,
  pendingLoads,
  removeErrors,
  loadErrors,
  onLoad,
  onUnload,
  onRemove,
  onDismissRemoveError,
  onDismissLoadError,
  onParams,
  onCancel,
  onSwitchToRecommended,
}: {
  modelsByKind: Record<ModelTaskKind, LocalModel[]>;
  loadedModelIds: Set<string>;
  loadedByKind: Record<ModelTaskKind, LoadedModel[]>;
  localHfIds: Set<string>;
  activeDownloads: DownloadProgress[];
  pendingLoads: Set<string>;
  removeErrors: Record<string, string>;
  loadErrors: Record<string, string>;
  onLoad: (id: string) => void;
  onUnload: (id: string) => void;
  onRemove: (id: string) => void;
  onDismissRemoveError: (id: string) => void;
  onDismissLoadError: (id: string) => void;
  onParams: (id: string, hfId: string | null) => void;
  onCancel: (id: string) => Promise<void>;
  onSwitchToRecommended: () => void;
}) {
  const totalLocal = Object.values(modelsByKind).reduce(
    (sum, arr) => sum + arr.length,
    0,
  );

  if (totalLocal === 0 && activeDownloads.length === 0) {
    return <EmptyLibrary onBrowseRecommended={onSwitchToRecommended} />;
  }

  return (
    <>
      {CATEGORY_ORDER.map((kind) => {
        const items = modelsByKind[kind];
        const loaded = loadedByKind[kind];
        const meta = CATEGORY_META[kind];

        // Hide empty categories in library view
        if (items.length === 0 && activeDownloads.length === 0) return null;

        // Also hide if the only items are active downloads for a different category
        const relevantDownloads = activeDownloads.filter(
          (d) => !localHfIds.has(d.model_id),
        );
        const hasContent =
          items.length > 0 ||
          relevantDownloads.some(
            (d) =>
              (pipelineTagToTaskKind(d.model_id as never) ??
                "text_generation") === kind,
          );

        if (!hasContent && relevantDownloads.length === 0) return null;

        return (
          <CategorySection
            key={kind}
            meta={meta}
            loadedCount={loaded.length}
            localCount={items.length}
          >
            {/* Active downloads at the top */}
            {relevantDownloads
              .filter(() => kind === "text_generation" || items.length > 0)
              .map((dl) => (
                <DownloadingCard
                  key={dl.model_id}
                  dl={dl}
                  onCancel={onCancel}
                />
              ))}

            {items.map((m) => {
              const isLoaded =
                loadedModelIds.has(m.id) ||
                (m.hf_id != null && loadedModelIds.has(m.hf_id));
              return (
                <LocalCard
                  key={m.id}
                  m={m}
                  loaded={isLoaded}
                  loading={pendingLoads.has(m.hf_id ?? m.id)}
                  removeError={removeErrors[m.id] ?? null}
                  loadError={loadErrors[m.hf_id ?? m.id] ?? null}
                  onDismissError={() => onDismissRemoveError(m.id)}
                  onDismissLoadError={() => onDismissLoadError(m.hf_id ?? m.id)}
                  onParams={() => onParams(m.id, m.hf_id)}
                  onLoad={() => onLoad(m.hf_id ?? m.id)}
                  onUnload={() => onUnload(m.hf_id ?? m.id)}
                  onRemove={() => onRemove(m.id)}
                />
              );
            })}
          </CategorySection>
        );
      })}
    </>
  );
}

// ─── recommended view ───────────────────────────────────────────────

function RecommendedView({
  localHfIds,
  loadedModelIds,
  downloads,
  onSmartDownload,
  onConfigure,
  onCancel,
}: {
  localHfIds: Set<string>;
  loadedModelIds: Set<string>;
  downloads: Record<string, DownloadProgress>;
  onSmartDownload: (hfId: string) => void;
  onConfigure: (hfId: string) => void;
  onCancel: (id: string) => Promise<void>;
}) {
  // Fetch spec-ranked recommendations for every task category.
  const recommendModels = useSystemStore((s) => s.recommendModels);
  const [specMap, setSpecMap] = useState<RecommendationMap | null>(null);

  useEffect(() => {
    let cancelled = false;
    recommendModels().then((map) => {
      if (!cancelled && Object.keys(map).length > 0) setSpecMap(map);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <>
      {CATEGORY_ORDER.map((kind) => {
        // Prefer dynamic spec-ranked recs; fall back to static.
        const recs: RecommendedModel[] =
          specMap?.[kind] ?? RECOMMENDED_BY_KIND[kind];
        if (recs.length === 0) return null;
        const meta = CATEGORY_META[kind];
        const installedCount = recs.filter((r) =>
          localHfIds.has(r.hfId),
        ).length;

        return (
          <CategorySection
            key={kind}
            meta={meta}
            loadedCount={recs.filter((r) => loadedModelIds.has(r.hfId)).length}
            localCount={installedCount}
          >
            {recs.map((rec) => {
              const isInstalled = localHfIds.has(rec.hfId);
              const isLoaded = loadedModelIds.has(rec.hfId);
              const dl = downloads[rec.hfId];
              const inFlight =
                dl &&
                (dl.state === "pending" ||
                  dl.state === "downloading" ||
                  dl.state === "verifying");

              if (inFlight) {
                return (
                  <DownloadingCard key={rec.hfId} dl={dl} onCancel={onCancel} />
                );
              }

              return (
                <RecommendedCard
                  key={rec.hfId}
                  rec={rec}
                  installed={isInstalled}
                  loaded={isLoaded}
                  onSmartDownload={() => onSmartDownload(rec.hfId)}
                  onConfigure={() => onConfigure(rec.hfId)}
                />
              );
            })}
          </CategorySection>
        );
      })}
    </>
  );
}

// ─── category section ───────────────────────────────────────────────

const CATEGORY_ICONS: Record<ModelTaskKind, React.ReactNode> = {
  text_generation: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v10Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  ),
  embeddings: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <rect
        x="3"
        y="3"
        width="7"
        height="7"
        rx="1.5"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <rect
        x="14"
        y="3"
        width="7"
        height="7"
        rx="1.5"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <rect
        x="3"
        y="14"
        width="7"
        height="7"
        rx="1.5"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <rect
        x="14"
        y="14"
        width="7"
        height="7"
        rx="1.5"
        stroke="currentColor"
        strokeWidth="1.6"
      />
    </svg>
  ),
  rerank: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M8 3v18M8 3l-4 4M8 3l4 4M16 21V3M16 21l4-4M16 21l-4-4"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  ),
  image_generation: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <rect
        x="3"
        y="3"
        width="18"
        height="18"
        rx="2"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <circle cx="8.5" cy="8.5" r="1.5" fill="currentColor" />
      <path
        d="m21 15-5-5L5 21"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  ),
  text2speech: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M11 5 6 9H2v6h4l5 4V5Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  ),
  speech2text: (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M19 10v2a7 7 0 0 1-14 0v-2"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M12 19v4M8 23h8"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  ),
};

function CategorySection({
  meta,
  loadedCount,
  localCount,
  children,
}: {
  meta: { label: string; blurb: string; kind: ModelTaskKind };
  loadedCount: number;
  localCount: number;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-2 flex items-center gap-2 border-b border-tui-border/40 pb-2 px-0.5">
        <span className="text-tui-fg-muted">{CATEGORY_ICONS[meta.kind]}</span>
        <h2
          className="text-[14px] font-semibold text-tui-fg"
          style={{ fontFamily: "var(--font-display)" }}
        >
          {meta.label}
        </h2>
        <span className="text-[11px] text-tui-fg-muted">
          · {loadedCount} loaded · {localCount} local
        </span>
      </div>
      <p className="mb-2.5 px-0.5 text-[11px] text-tui-fg-muted">
        {meta.blurb}
      </p>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
        {children}
      </div>
    </section>
  );
}

// ─── search bar ─────────────────────────────────────────────────────

const SearchBox = forwardRef<
  HTMLInputElement,
  {
    value: string;
    onChange: (v: string) => void;
    onFocus: () => void;
    searching: boolean;
    placeholder: string;
  }
>(function SearchBox(
  { value, onChange, onFocus, searching, placeholder },
  ref,
) {
  return (
    <div className="relative">
      <span
        className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-tui-fg-muted"
        aria-hidden="true"
      >
        {searching ? (
          <Spinner size="sm" />
        ) : (
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
            <circle
              cx="11"
              cy="11"
              r="7"
              stroke="currentColor"
              strokeWidth="1.6"
            />
            <path
              d="m21 21-4.3-4.3"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
          </svg>
        )}
      </span>
      <input
        ref={ref}
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={onFocus}
        placeholder={placeholder}
        autoComplete="off"
        className={
          "allow-select h-11 w-full rounded-[8px] " +
          "border border-tui-border bg-[var(--fluent-bg-subtle)] " +
          "border-b-[var(--fluent-stroke-strong)] " +
          "pl-10 pr-3 text-[14px] text-tui-fg " +
          "placeholder:text-tui-fg-muted " +
          "transition-all duration-150 " +
          "hover:bg-[var(--fluent-bg-subtle-hover)] " +
          "focus:outline-none focus:border-b-2 focus:border-b-tui-accent " +
          "focus:bg-[var(--fluent-bg-subtle-hover)] " +
          "focus:shadow-[var(--fluent-shadow-4)] focus:pb-[1px]"
        }
      />
    </div>
  );
});

// ─── search results popover ─────────────────────────────────────────

function SearchResultsPopover({
  items,
  searching,
  localIds,
  downloads,
  onDownload,
  onConfigure,
  onUpdate,
  onCancel,
}: {
  items: HfModelSummary[];
  searching: boolean;
  localIds: Set<string>;
  downloads: Record<string, DownloadProgress>;
  onDownload: (id: string) => void;
  onConfigure: (id: string) => void;
  onUpdate: (id: string) => void;
  onCancel: (id: string) => void;
}) {
  return (
    <div
      role="listbox"
      aria-label="OpenVINO model search results"
      className={
        "fluent-mica absolute top-full right-0 left-0 z-30 mt-2 " +
        "overflow-hidden rounded-[8px] border border-tui-border " +
        "shadow-[var(--fluent-shadow-16)] " +
        "fluent-popover-in"
      }
    >
      {items.length === 0 ? (
        <div className="flex items-center gap-2 px-4 py-3 text-[12px] text-tui-fg-muted">
          {searching ? (
            <>
              <Spinner size="sm" /> Searching OpenVINO models…
            </>
          ) : (
            <>No OpenVINO results. Try a different term.</>
          )}
        </div>
      ) : (
        <ul className="max-h-[420px] overflow-auto">
          {items.map((m) => {
            const dl = downloads[m.id];
            const inFlight =
              dl &&
              (dl.state === "pending" ||
                dl.state === "downloading" ||
                dl.state === "verifying");
            const isLocal = localIds.has(m.id);
            return (
              <li
                key={m.id}
                className="fluent-reveal border-b border-tui-border last:border-b-0 transition-colors duration-150 hover:bg-[var(--fluent-bg-subtle-hover)]"
              >
                <div className="flex items-start justify-between gap-3 px-3 py-2.5">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 truncate">
                      <span className="truncate text-[13px] font-medium text-tui-fg">
                        {m.id}
                      </span>
                      <span className="shrink-0 rounded-[3px] border border-tui-accent/40 bg-tui-selection px-1.5 py-px text-[10px] font-medium text-tui-accent">
                        IR
                      </span>
                    </div>
                    <div className="mt-0.5 text-[11px] text-tui-fg-muted">
                      {m.downloads.toLocaleString()} downloads · {m.likes} likes
                      · {m.pipeline_tag ?? "—"} ·{" "}
                      {m.total_size_bytes ? bytes(m.total_size_bytes) : "?"} ·{" "}
                      {relativeTime(m.updated_at)}
                    </div>
                  </div>
                  <div className="flex shrink-0 gap-1.5">
                    {inFlight ? (
                      <TuiButton
                        variant="danger"
                        onClick={() => onCancel(m.id)}
                      >
                        Cancel
                      </TuiButton>
                    ) : isLocal ? (
                      <TuiButton variant="ghost" onClick={() => onUpdate(m.id)}>
                        Update
                      </TuiButton>
                    ) : (
                      <>
                        <TuiButton
                          variant="ghost"
                          onClick={() => onConfigure(m.id)}
                          title="Pick task / configure deploy params before download"
                        >
                          Configure…
                        </TuiButton>
                        <TuiButton
                          variant="primary"
                          onClick={() => onDownload(m.id)}
                        >
                          Download
                        </TuiButton>
                      </>
                    )}
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

// ─── shared card shell ──────────────────────────────────────────────

function CardShell({
  children,
  highlight = false,
}: {
  children: React.ReactNode;
  highlight?: boolean;
}) {
  return (
    <div
      className={
        "fluent-mica fluent-reveal group flex min-w-0 flex-col gap-2 rounded-[10px] " +
        "border border-tui-border p-3 " +
        "shadow-[var(--fluent-shadow-2)] " +
        "transition-all duration-200 ease-out " +
        "hover:shadow-[var(--fluent-shadow-4)] " +
        (highlight
          ? "border-tui-accent/40 bg-[var(--fluent-bg-subtle)] "
          : "hover:bg-[var(--fluent-bg-subtle)] ")
      }
    >
      {children}
    </div>
  );
}

// ─── downloading card ───────────────────────────────────────────────

function DownloadingCard({
  dl,
  onCancel,
}: {
  dl: DownloadProgress;
  onCancel: (id: string) => Promise<void>;
}) {
  const label = `${dl.files_done}/${dl.files_total} · ${bytes(dl.bytes_done)}${
    dl.bytes_total ? ` / ${bytes(dl.bytes_total)}` : ""
  }`;
  return (
    <CardShell highlight>
      <div className="flex items-start justify-between gap-2">
        <span
          className="truncate text-[13px] font-medium text-tui-fg"
          title={dl.model_id}
        >
          {dl.model_id}
        </span>
        <TuiButton variant="danger" onClick={() => void onCancel(dl.model_id)}>
          Cancel
        </TuiButton>
      </div>
      <div className="flex items-center gap-2 text-[11px] text-tui-accent">
        <Spinner size="sm" />
        <span className="capitalize">{dl.state}</span>
      </div>
      <ProgressBar
        value={dl.bytes_done}
        max={dl.bytes_total ?? dl.bytes_done + 1}
        label={label}
        tone="ok"
      />
    </CardShell>
  );
}

// ─── local card ─────────────────────────────────────────────────────

function LocalCard({
  m,
  loaded,
  loading,
  removeError,
  loadError,
  onParams,
  onLoad,
  onUnload,
  onRemove,
  onDismissError,
  onDismissLoadError,
}: {
  m: LocalModel;
  loaded: boolean;
  loading: boolean;
  removeError: string | null;
  loadError: string | null;
  onParams: () => void;
  onLoad: () => void;
  onUnload: () => void;
  onRemove: () => void;
  onDismissError: () => void;
  onDismissLoadError: () => void;
}) {
  const verifyMeta = (() => {
    const verified = m.verified_files;
    const total = m.files;
    if (verified == null || total == null || total === 0) {
      return {
        label: "Unverified",
        title: "no sha256 manifest yet — re‑download to verify integrity",
        className: "text-tui-fg-muted",
      };
    }
    if (verified >= total) {
      return {
        label: `Verified ${verified}/${total}`,
        title: `all ${total} files verified against HF sha256 at download time`,
        className: "text-tui-accent",
      };
    }
    if (verified === 0) {
      return {
        label: `0/${total} verified`,
        title:
          "upstream published no sha256 for these files (likely a small non‑LFS repo)",
        className: "text-tui-warn",
      };
    }
    return {
      label: `${verified}/${total} verified`,
      title: `${total - verified} file(s) have no upstream sha256 to check against`,
      className: "text-tui-warn",
    };
  })();

  return (
    <CardShell>
      {/* Row 1: model name — full width, never squished */}
      <div className="min-w-0">
        <p
          className="text-[13px] font-medium text-tui-fg leading-tight"
          title={m.hf_id ?? m.id}
        >
          {m.hf_id ?? m.id}
        </p>
        <p className="mt-0.5 text-[10px] uppercase tracking-wide text-tui-fg-muted">
          {m.pipeline_tag
            ? m.pipeline_tag.replace(/-/g, " ")
            : "text generation"}
        </p>
      </div>

      {/* Row 2: metadata */}
      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-tui-fg-muted">
        <span>{bytes(m.bytes)}</span>
        {m.files != null && <span>· {m.files} files</span>}
        {m.revision && <span>· @{m.revision.slice(0, 7)}</span>}
        <span>· {relativeTime(m.added_at)}</span>
        <span className={verifyMeta.className}>· {verifyMeta.label}</span>
      </div>

      {/* Row 3: action buttons */}
      <div className="mt-1 flex items-center gap-1.5">
        {loaded && <ModelStatusPopover modelId={m.hf_id ?? m.id} />}
        {loaded ? (
          <button
            type="button"
            onClick={onUnload}
            disabled={loading}
            title="Remove this model from the OVMS serving set"
            className={
              "group/unload inline-flex flex-1 items-center justify-center gap-1.5 rounded-[4px] " +
              "border border-tui-accent/40 bg-[rgba(96,205,255,0.10)] " +
              "px-2 py-[5px] text-[11px] font-medium " +
              "text-tui-accent transition-colors " +
              (loading
                ? "cursor-progress opacity-70"
                : "hover:border-tui-err/40 hover:bg-[rgba(255,153,164,0.10)] hover:text-tui-err")
            }
          >
            {loading ? (
              <>
                <Spinner size="sm" /> Unloading…
              </>
            ) : (
              <>
                <span
                  aria-hidden="true"
                  className={
                    "h-1.5 w-1.5 rounded-full bg-tui-accent " +
                    "transition-colors group-hover/unload:bg-tui-err"
                  }
                />
                <span className="group-hover/unload:hidden">Loaded</span>
                <span className="hidden group-hover/unload:inline">Unload</span>
              </>
            )}
          </button>
        ) : (
          <TuiButton
            size="sm"
            variant="primary"
            onClick={onLoad}
            disabled={loading}
            title="Load model in OVMS"
            className="flex-1"
          >
            {loading ? (
              <>
                <Spinner size="sm" /> Loading…
              </>
            ) : (
              "Load"
            )}
          </TuiButton>
        )}
        <button
          onClick={onParams}
          className="rounded-[4px] px-2 py-[5px] text-[11px] text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-accent"
          title="Edit task / deploy params"
        >
          Params
        </button>
        <button
          onClick={loaded ? undefined : onRemove}
          disabled={loaded}
          className={
            "rounded p-1.5 transition-colors " +
            (loaded
              ? "cursor-not-allowed text-tui-fg-muted opacity-50"
              : "text-tui-fg-muted hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-err")
          }
          title={
            loaded
              ? "Currently loaded into OVMS — click Unload first"
              : "Remove"
          }
          aria-label={
            loaded ? "Remove (disabled while loaded)" : "Remove model"
          }
        >
          <svg
            width="13"
            height="13"
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

      {/* Error banners */}
      {removeError && (
        <div className="mt-1 flex items-start justify-between gap-2 rounded-[6px] border border-tui-err/40 bg-[rgba(255,153,164,0.08)] px-2.5 py-1.5 text-[11px] text-tui-err shadow-[var(--fluent-shadow-2)]">
          <span className="min-w-0 break-words">
            <span className="font-medium">Couldn't remove:</span> {removeError}
          </span>
          <button
            type="button"
            onClick={onDismissError}
            className="shrink-0 rounded-[4px] px-1 py-px text-tui-err/80 transition-colors hover:bg-[rgba(255,153,164,0.12)] hover:text-tui-err"
            aria-label="Dismiss error"
          >
            ✕
          </button>
        </div>
      )}
      {loadError && (
        <div className="mt-1 flex items-start justify-between gap-2 rounded-[6px] border border-tui-err/40 bg-[rgba(255,153,164,0.08)] px-2.5 py-1.5 text-[11px] text-tui-err shadow-[var(--fluent-shadow-2)]">
          <span className="min-w-0 break-words">
            <span className="font-medium">OVMS error:</span> {loadError}
          </span>
          <button
            type="button"
            onClick={onDismissLoadError}
            className="shrink-0 rounded-[4px] px-1 py-px text-tui-err/80 transition-colors hover:bg-[rgba(255,153,164,0.12)] hover:text-tui-err"
            aria-label="Dismiss error"
          >
            ✕
          </button>
        </div>
      )}
    </CardShell>
  );
}

// ─── model status popover ───────────────────────────────────────────

function ModelStatusPopover({ modelId }: { modelId: string }) {
  const getModelStatus = useServerStore((s) => s.getModelStatus);
  const [status, setStatus] = useState<TfsModelStatus | null>(null);
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const anchorRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  // Close on outside click / Escape
  useEffect(() => {
    if (!open) return;
    function onPointer(e: MouseEvent) {
      if (
        popoverRef.current?.contains(e.target as Node) ||
        anchorRef.current?.contains(e.target as Node)
      )
        return;
      setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  async function fetchStatus() {
    setFetching(true);
    setError(null);
    try {
      const tfs = await getModelStatus(modelId);
      setStatus(tfs);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setStatus(null);
    } finally {
      setFetching(false);
    }
  }

  function handleClick() {
    if (open) {
      setOpen(false);
      return;
    }
    setOpen(true);
    if (!status && !error) void fetchStatus();
  }

  const versions = status?.model_version_status ?? [];

  return (
    <div className="relative">
      <button
        ref={anchorRef}
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          handleClick();
        }}
        className={
          "rounded-[4px] p-1.5 transition-colors " +
          "text-tui-fg-muted " +
          "hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-accent " +
          (open ? "bg-[var(--fluent-bg-subtle)] text-tui-accent" : "")
        }
        title="View OVMS serving status"
        aria-label="View model status"
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
        </svg>
      </button>

      {open && (
        <div
          ref={popoverRef}
          className={
            "fluent-mica absolute bottom-full left-0 z-40 mb-2 " +
            "w-64 overflow-hidden rounded-[8px] border border-tui-border " +
            "shadow-[var(--fluent-shadow-16)] fluent-popover-in"
          }
        >
          {/* Header */}
          <div className="flex items-center justify-between gap-2 border-b border-tui-border px-3 py-2">
            <span className="truncate text-[12px] font-medium text-tui-fg">
              {modelId}
            </span>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void fetchStatus();
              }}
              disabled={fetching}
              className="shrink-0 rounded-[3px] p-0.5 text-tui-fg-muted transition-colors hover:bg-[var(--fluent-bg-subtle-hover)] hover:text-tui-fg disabled:opacity-50"
              title="Refresh status"
              aria-label="Refresh status"
            >
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2" />
              </svg>
            </button>
          </div>

          {/* Body */}
          <div className="max-h-[260px] overflow-auto">
            {fetching && (
              <div className="flex items-center gap-2 px-3 py-4 text-[11px] text-tui-fg-muted">
                <Spinner size="sm" /> Fetching from OVMS…
              </div>
            )}

            {error && !fetching && (
              <div className="px-3 py-3 text-[11px] text-tui-err">{error}</div>
            )}

            {!fetching && !error && versions.length === 0 && (
              <div className="px-3 py-3 text-[11px] text-tui-fg-muted">
                No version status available.
              </div>
            )}

            {!fetching &&
              !error &&
              versions.map((v) => {
                const stateColor = (() => {
                  switch (v.state) {
                    case "AVAILABLE":
                      return "text-tui-ok";
                    case "LOADING":
                    case "START":
                      return "text-tui-warn";
                    default:
                      return "text-tui-err";
                  }
                })();
                const hasErr =
                  v.status?.error_code && v.status.error_code !== "OK";
                return (
                  <div
                    key={v.version}
                    className="border-b border-tui-border px-3 py-2 last:border-b-0"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-[11px] font-medium text-tui-fg">
                        v{v.version}
                      </span>
                      <span
                        className={`shrink-0 rounded-[3px] border border-current/30 px-1.5 py-px text-[10px] font-medium uppercase ${stateColor}`}
                      >
                        {v.state.toLowerCase()}
                      </span>
                    </div>
                    {hasErr && (
                      <div className="mt-1 text-[10px] text-tui-err">
                        {v.status.error_code}: {v.status.error_message}
                      </div>
                    )}
                  </div>
                );
              })}
          </div>
        </div>
      )}
    </div>
  );
}

// ─── recommended card ───────────────────────────────────────────────

function RecommendedCard({
  rec,
  installed,
  loaded,
  onSmartDownload,
  onConfigure,
}: {
  rec: RecommendedModel;
  installed: boolean;
  loaded: boolean;
  onSmartDownload: () => void;
  onConfigure: () => void;
}) {
  return (
    <CardShell highlight={installed}>
      {/* Row 1: name + status badges */}
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <p
            className="truncate text-[13px] font-medium text-tui-fg leading-tight"
            title={rec.hfId}
          >
            {rec.hfId}
          </p>
          <div className="flex shrink-0 items-center gap-1">
            {installed && (
              <span className="rounded-[3px] border border-tui-accent/40 bg-tui-accent/10 px-1.5 py-px text-[10px] font-medium text-tui-accent">
                Installed
              </span>
            )}
            {loaded && (
              <span
                className="inline-flex items-center gap-1 rounded-[3px] border border-tui-accent/50 bg-tui-selection px-1.5 py-px text-[10px] font-medium text-tui-accent"
                title="This model is currently loaded into OVMS"
              >
                <span className="h-1.5 w-1.5 rounded-full bg-tui-accent" />
                Loaded
              </span>
            )}
          </div>
        </div>
        <p className="mt-1 text-[11px] text-tui-fg-muted leading-relaxed">
          {rec.blurb}
        </p>
        <div className="mt-0.5 flex items-center gap-1.5 text-[10px] text-tui-fg-dim">
          <span>{rec.sizeHint}</span>
        </div>
      </div>

      {/* Row 2: action buttons */}
      <div className="mt-1 flex items-center gap-1.5">
        {installed ? (
          <TuiButton
            variant="ghost"
            size="sm"
            onClick={onSmartDownload}
            title="Re‑download / update this model"
            className="flex-1"
          >
            Re‑download
          </TuiButton>
        ) : (
          <>
            <TuiButton
              variant="ghost"
              size="sm"
              onClick={onConfigure}
              title="Configure task / params before downloading"
            >
              Configure…
            </TuiButton>
            <TuiButton
              variant="primary"
              size="sm"
              onClick={onSmartDownload}
              className="flex-1"
            >
              Download
            </TuiButton>
          </>
        )}
      </div>
    </CardShell>
  );
}

// ─── empty library state ────────────────────────────────────────────

function EmptyLibrary({
  onBrowseRecommended,
}: {
  onBrowseRecommended: () => void;
}) {
  return (
    <div className="mx-auto flex max-w-md flex-1 flex-col items-center justify-center text-center">
      <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-tui-accent-dim text-white shadow-[var(--fluent-shadow-8)]">
        <svg
          width="22"
          height="22"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M12 3 3 7.5 12 12l9-4.5L12 3Z" />
          <path d="M3 12.5 12 17l9-4.5" />
          <path d="M3 17.5 12 22l9-4.5" />
        </svg>
      </div>
      <h2
        className="text-[15px] font-semibold text-tui-fg"
        style={{ fontFamily: "var(--font-display)" }}
      >
        No local models yet
      </h2>
      <p className="mt-1 text-[12px] text-tui-fg-muted">
        Search the bar above and hit{" "}
        <span className="text-tui-fg">Download</span> to install an OpenVINO IR
        model.
      </p>
      <button
        type="button"
        onClick={onBrowseRecommended}
        className={
          "mt-4 rounded-lg border border-tui-accent/40 " +
          "bg-[var(--fluent-bg-subtle)] px-4 py-2 " +
          "text-[12px] font-medium text-tui-accent " +
          "shadow-[var(--fluent-shadow-2)] " +
          "transition-all duration-150 " +
          "hover:border-tui-accent hover:bg-tui-accent/10 " +
          "hover:shadow-[var(--fluent-shadow-4)] " +
          "active:scale-[0.98]"
        }
      >
        Browse recommended models
      </button>
    </div>
  );
}
