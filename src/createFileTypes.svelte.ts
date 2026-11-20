import { onDestroy, untrack } from "svelte";
import { invoke } from "@tauri-apps/api/core";
import type { NodeView, ScanSummary } from "./types";
import type { FileTypeStats } from "./fileTypes";
import { latestOnly } from "./latestQuery";

export type FileTypeScope = "scan" | "folder";

/** Queries only scan metadata; nothing is enumerated on the filesystem. */
export default function createFileTypes(context: {
  readonly summary: ScanSummary | null;
  readonly focus: NodeView | null;
  readonly nodes: NodeView[];
  readonly enabled: boolean;
}) {
  let scope = $state<FileTypeScope>("scan");
  const queryStats = latestOnly(),
    queryHighlights = latestOnly();
  let choice = $state.raw<{ scanId: string; extension: string } | null>(null);
  let retry = $state(0);
  // Stats are keyed by scan and folder; `version` (revision and retry) says how fresh they are, so
  // the last ones stay on screen while a newer revision loads.
  type Loaded = { key: string; version: string; stats: FileTypeStats };
  let loaded = $state.raw<Loaded | null>(null);
  let scanLoaded = $state.raw<Loaded | null>(null);
  let disposed = false;
  onDestroy(() => {
    disposed = true;
  });
  let failure = $state.raw<{ key: string; message: string } | null>(null);
  let highlighted = $state.raw<{ key: string; ids: number[] } | null>(null);
  let highlightFailure = $state.raw<{ key: string; message: string } | null>(
    null,
  );
  const scanId = $derived(context.summary?.scanId);
  const nodeId = $derived(
    scope === "folder" ? context.focus?.id : context.summary?.rootId,
  );
  const extension = $derived(
    choice?.scanId === scanId ? (choice?.extension ?? null) : null,
  );
  // Live scans refresh at most once per ten seconds, plus every final state: each query walks the
  // whole tree under the scan's read lock.
  const revision = $derived(
    `${context.summary?.state}:${Math.floor((context.summary?.elapsedMs ?? 0) / 10000)}`,
  );
  const running = $derived(context.summary?.state === "running");
  const statsScope = $derived(`${scanId}:${nodeId}`);
  const version = $derived(`${revision}:${retry}`);
  const statsKey = $derived(`${statsScope}:${version}`);
  const highlightKey = $derived(`${statsScope}:${JSON.stringify(extension)}`);
  const stats = $derived(
    (loaded?.key === statsScope
      ? loaded
      : scanLoaded?.key === statsScope
        ? scanLoaded
        : null
    )?.stats ?? null,
  );
  const error = $derived(failure?.key === statsKey ? failure.message : "");
  // Ids of tiles no longer shown are harmless: the map only matches tiles it has.
  const highlights = $derived(
    highlighted?.key === highlightKey ? highlighted.ids : [],
  );
  const highlightError = $derived(
    highlightFailure?.key === highlightKey ? highlightFailure.message : "",
  );
  const highlightsLoading = $derived(
    extension !== null && !highlightError && highlighted?.key !== highlightKey,
  );

  $effect(() => {
    const enabled = context.enabled;
    const key = statsKey;
    const scope = statsScope;
    const fresh = version;
    const scan = scanId;
    const root = nodeId;
    if (!enabled || scan === undefined || root === undefined) return;
    const cached = () =>
      [loaded, scanLoaded].some(
        (entry) => entry?.key === scope && entry.version === fresh,
      );
    if (untrack(cached)) return;
    let ignore = false;
    queryStats(async () => {
      // A previous in-flight query may fill the cache while this job is queued.
      if (ignore || disposed || cached()) return;
      await invoke<FileTypeStats>("get_file_types", {
        scanId: scan,
        nodeId: root,
      })
        .then((result) => {
          // Cache a valid in-flight result even when its panel has just been hidden.
          if (disposed || scanId !== scan) return;
          if (failure?.key === key) failure = null;
          const entry = { key: scope, version: fresh, stats: result };
          if (root === context.summary?.rootId) scanLoaded = entry;
          else loaded = entry;
        })
        .catch((reason) => {
          if (!ignore) failure = { key, message: String(reason) };
        });
    });
    return () => {
      ignore = true;
    };
  });

  $effect(() => {
    const key = highlightKey;
    const scan = scanId;
    const root = nodeId;
    const selected = extension;
    // A live scan's map reloads every second or two, but each query walks the whole tree under the
    // scan's read lock: while running, re-query only when the revision (or a retry) changes.
    void version;
    const nodes = running ? untrack(() => context.nodes) : context.nodes;
    if (scan === undefined || root === undefined || selected === null) return;
    const nodeIds = nodes
      .filter((node) => node.kind !== "aggregate")
      .map((node) => node.id);
    untrack(() => (highlightFailure = null));
    let ignore = false;
    queryHighlights(async () => {
      if (ignore) return;
      await invoke<number[]>("get_file_type_highlights", {
        scanId: scan,
        nodeId: root,
        extension: selected,
        nodeIds,
      })
        .then((ids) => {
          if (!ignore) highlighted = { key, ids };
        })
        .catch((reason) => {
          if (!ignore) highlightFailure = { key, message: String(reason) };
        });
    });
    return () => {
      ignore = true;
    };
  });

  // Release references to a previous scan, including potentially large maps.
  $effect(() => {
    const scan = scanId;
    untrack(() => {
      if (choice && choice.scanId !== scan) choice = null;
      loaded = null;
      scanLoaded = null;
      failure = null;
      highlighted = null;
      highlightFailure = null;
    });
  });

  return {
    get scope() {
      return scope;
    },
    setScope(value: FileTypeScope) {
      scope = value;
    },
    get nodeId() {
      return nodeId;
    },
    get extension() {
      return extension;
    },
    select(value: string | null) {
      choice =
        value === null || scanId === undefined
          ? null
          : { scanId, extension: value };
    },
    get stats() {
      return stats;
    },
    get loading() {
      return context.enabled && !stats && !error;
    },
    get error() {
      return error;
    },
    get highlights() {
      return highlights;
    },
    get highlightsLoading() {
      return highlightsLoading;
    },
    get highlightError() {
      return highlightError;
    },
    retry() {
      retry++;
    },
  };
}
