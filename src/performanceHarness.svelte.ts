import { onMount, untrack } from "svelte";
import { emit, listen } from "@tauri-apps/api/event";
import { canvasEvidence, runNativeInteractions } from "./nativeBenchmark";
import type { FullDiskAccess, NodeView, ScanSummary, Target } from "./types";

declare global {
  interface Window {
    __PERFORMANCE_BENCHMARK__?: {
      root: string;
      mode: string;
      interactions?: boolean;
    };
  }
}

/** What the benchmark harness reads from and does to the app. */
export interface PerformanceApp {
  readonly map: NodeView[];
  readonly summary: ScanSummary | null;
  readonly focus: NodeView | null;
  readonly listLoading: boolean;
  readonly targets: Target[];
  readonly access: FullDiskAccess | null;
  readonly sortReady: boolean;
  readonly busy: boolean;
  readonly liveTarget: Target | null;
  readonly activeId: { current: string | null };
  setTarget(target: Target): void;
  start(target: Target | null): Promise<void>;
  setError(message: string): void;
}

/**
 * Drives a performance benchmark build (`VITE_PERFORMANCE_BENCHMARK=1`): starts the configured scan, rescans
 * once when asked, and reports when the completed map has drawn. Call it during component setup.
 */
export function createPerformanceHarness(app: PerformanceApp) {
  let config = $state.raw<Window["__PERFORMANCE_BENCHMARK__"] | null>(null);
  const started = { current: false };
  const reported = { current: false };
  const rescanned = { current: false };
  let drawnMap = $state.raw<NodeView[] | null>(null);
  let progressUpdates = 0;
  let mapReady = $state.raw(false);
  let mapQueryPending = $state.raw(false);
  let eventsReady = $state.raw(false);
  let mapScan = $state.raw<string | null>(null);
  const rescanMode = () =>
    ["same_process", "changed"].includes(config?.mode ?? "");

  onMount(() => {
    const receive = () => (config = window.__PERFORMANCE_BENCHMARK__ ?? null);
    receive();
    window.addEventListener("performance-config", receive);
    return () => window.removeEventListener("performance-config", receive);
  });
  onMount(() => {
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    void listen<{
      scanId: string;
    }>("performance-rescan-ready", ({ payload }) => {
      if (
        cancelled ||
        rescanned.current ||
        !rescanMode() ||
        !reported.current ||
        app.activeId.current !== payload.scanId
      )
        return;
      rescanned.current = true;
      app.activeId.current = null;
      reported.current = false;
      void app.start(app.liveTarget);
    })
      .then((off) => {
        if (cancelled) off();
        else {
          unsubscribe = off;
          eventsReady = true;
        }
      })
      .catch((e) => {
        if (!cancelled) app.setError(String(e));
      });
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  });
  $effect(() => {
    [config, eventsReady, app.targets, app.access, app.sortReady, app.busy];
    return untrack(() => {
      if (
        !eventsReady ||
        !config ||
        started.current ||
        !app.access ||
        !app.sortReady ||
        app.busy
      )
        return;
      const location = app.targets.find((item) => item.path === config!.root);
      if (!location) return;
      started.current = true;
      app.setTarget(location);
      void app.start(location);
    });
  });
  // The list can finish after the canvas draws. Retain the current draw identity
  // so reporting does not depend on repainting for unrelated query state.
  $effect(() => {
    const drawn = drawnMap;
    const loading = app.listLoading;
    untrack(() => {
      if (drawn && !loading) void finishRender(drawn);
    });
  });
  async function finishRender(nodes: NodeView[]) {
    const summary = app.summary;
    if (
      reported.current ||
      !summary ||
      app.activeId.current !== summary.scanId ||
      summary.state !== "completed" ||
      summary.scanPhase !== "done" ||
      mapScan !== summary.scanId ||
      nodes !== app.map ||
      app.listLoading ||
      mapQueryPending
    )
      return;
    reported.current = true;
    const scanId = summary.scanId;
    const canvas = canvasEvidence(nodes, app.focus?.id ?? summary.rootId);
    const interactions =
      config?.interactions && (!rescanMode() || rescanned.current)
        ? await runNativeInteractions()
        : undefined;
    await emit("performance-rendered", {
      scanId,
      interactions,
      progressUpdates,
      canvas_evidence: canvas,
    });
  }

  return {
    get mapReady() {
      return mapReady && !mapQueryPending;
    },
    /** A scan start: the benchmark clock starts, and the start waits for it. */
    scanStarting: () => emit("performance-start"),
    /** A start took the view: nothing it drew or counted belongs to the new scan. */
    viewTaken() {
      drawnMap = null;
      mapReady = false;
      progressUpdates = 0;
    },
    progress(state: string) {
      if (state === "running") progressUpdates++;
    },
    mapQueryStarted() {
      mapQueryPending = true;
    },
    mapQueryEnded() {
      mapQueryPending = false;
    },
    /** The map query returned `nodes` for `snapshot`'s scan. */
    mapQueryResult(nodes: NodeView[], snapshot: ScanSummary) {
      // A worker can draw the old node array while the native query is
      // outstanding. Only the returned dataset can certify readiness.
      if (nodes !== drawnMap) {
        drawnMap = null;
        mapReady = false;
      }
      mapQueryPending = false;
      if (snapshot.state === "completed" && snapshot.scanPhase === "done")
        mapScan = snapshot.scanId;
    },
    layoutPending() {
      mapReady = false;
      drawnMap = null;
    },
    rendered(nodes: NodeView[]) {
      if (nodes === app.map) {
        drawnMap = nodes;
        mapReady = true;
      }
      void finishRender(nodes);
    },
  };
}
