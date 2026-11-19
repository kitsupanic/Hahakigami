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
  let loaded = $state.raw<{ key: string; stats: FileTypeStats } | null>(null);
  let scanLoaded = $state.raw<{ key: string; stats: FileTypeStats } | null>(
    null,
  );
  let disposed = false;
  onDestroy(() => {
    disposed = true;
  });
  let failure = $state.raw<{ key: string; message: string } | null>(null);
  let highlighted = $state.raw<{
    key: string;
    nodes: NodeView[];
    ids: number[];
  } | null>(null);
  let highlightFailure = $state.raw<{
    key: string;
    nodes: NodeView[];
    message: string;
  } | null>(null);
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
  const statsKey = $derived(`${scanId}:${nodeId}:${revision}:${retry}`);
  const highlightKey = $derived(`${statsKey}:${JSON.stringify(extension)}`);
  const stats = $derived(
    loaded?.key === statsKey
      ? loaded.stats
      : scanLoaded?.key === statsKey
        ? scanLoaded.stats
        : null,
  );
  const error = $derived(failure?.key === statsKey ? failure.message : "");
  const highlights = $derived(
    highlighted?.key === highlightKey && highlighted.nodes === context.nodes
      ? highlighted.ids
      : [],
  );
  const highlightError = $derived(
    highlightFailure?.key === highlightKey &&
      highlightFailure.nodes === context.nodes
      ? highlightFailure.message
      : "",
  );
  const highlightsLoading = $derived(
    extension !== null &&
      !highlightError &&
      !(
        highlighted?.key === highlightKey && highlighted.nodes === context.nodes
      ),
  );

  $effect(() => {
    const enabled = context.enabled;
    const key = statsKey;
    const scan = scanId;
    const root = nodeId;
    if (!enabled || scan === undefined || root === undefined) return;
    if (untrack(() => loaded?.key === key || scanLoaded?.key === key)) return;
    const version = revision;
    let ignore = false;
    queryStats(async () => {
      // A previous in-flight query may fill the cache while this job is queued.
      if (ignore || disposed || loaded?.key === key || scanLoaded?.key === key)
        return;
      await invoke<FileTypeStats>("get_file_types", {
        scanId: scan,
        nodeId: root,
      })
        .then((result) => {
          // Cache a valid in-flight result even when its panel has just been hidden.
          if (disposed || scanId !== scan || revision !== version) return;
          if (failure?.key === key) failure = null;
          if (root === context.summary?.rootId)
            scanLoaded = { key, stats: result };
          else loaded = { key, stats: result };
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
    const nodes = context.nodes;
    if (scan === undefined || root === undefined || selected === null) return;
    const nodeIds = nodes
      .filter((node) => node.kind !== "aggregate")
      .map((node) => node.id);
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
          if (!ignore) highlighted = { key, nodes, ids };
        })
        .catch((reason) => {
          if (!ignore)
            highlightFailure = { key, nodes, message: String(reason) };
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
