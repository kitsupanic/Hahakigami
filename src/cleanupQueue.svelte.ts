import { invoke } from "@tauri-apps/api/core";
import { untrack } from "svelte";
import type { CleanupReport, QueueEntry, QueueView } from "./cleanupTypes";
import type { NodeView, Target } from "./types";
import {
  createQueueMarker,
  normalizeQueuePath,
  queuePathKey,
} from "./queueMarkers";
import { bandSegments, bandSummary } from "./queueBands";
import { commonFolder, folderItems } from "./queueFolders";
import { compactBytes } from "./uiFormat";

/** A Cleanup queue review: the items the backend will move to the Trash, and the token that commits them. */
export type QueueReview = { token: string; items: NodeView[] };

/** What the Cleanup queue asks of the backend. Production wraps Tauri; tests pass a fake. */
export type QueuePort = {
  list(): Promise<QueueView>;
  add(scanId: string, nodeIds: number[]): Promise<QueueView>;
  remove(entryIds: string[]): Promise<QueueView>;
  clear(): Promise<QueueView>;
  /** The place to scan so an entry gets a Matched scan. */
  target(entryId: string): Promise<Target>;
  prepare(scanId: string): Promise<QueueReview>;
};

export const tauriQueuePort: QueuePort = {
  list: () => invoke<QueueView>("list_cleanup_queue"),
  add: (scanId, nodeIds) =>
    invoke<QueueView>("add_cleanup_queue", { scanId, nodeIds }),
  remove: (entryIds) =>
    invoke<QueueView>("remove_cleanup_queue", { entryIds }),
  clear: () => invoke<QueueView>("clear_cleanup_queue"),
  target: (entryId) => invoke<Target>("cleanup_queue_target", { entryId }),
  prepare: (scanId) =>
    invoke<QueueReview>("prepare_cleanup_queue", { scanId }),
};

/** Tells the open cleanup report how its queue check went. */
export type QueueReporter = (
  state: CleanupReport["queueState"],
  view?: QueueView,
  error?: string | null,
) => void;

export type CleanupQueueHooks = {
  /** The backend queue can be reached (not a browser preview or a benchmark build). */
  available(): boolean;
  /** The app allows queue changes now: no folder picker or cleanup in progress. */
  mutable(): boolean;
  /** Reports to the cleanup report open when the check starts, if any. */
  reporter(): QueueReporter;
  /** A change went through. */
  mutated(): void;
  /** The explorer selection. */
  selection(): readonly NodeView[];
  /** The scan on screen; null on Home or with none. */
  shownScanId(): string | null;
  /** A queue root's disk name, if the Disks list knows it. */
  diskName(root: string): string | undefined;
  /** Adds the explorer selection, behind the app's own gates. */
  addSelection(): Promise<unknown>;
};

/**
 * The Cleanup queue as the backend keeps it, with the explorer selection measured against it.
 * Must be created inside an effect root, such as a component initialising (it loads the queue then).
 */
export function createCleanupQueue(port: QueuePort, hooks: CleanupQueueHooks) {
  let view = $state.raw<QueueView>({
    entries: [],
    revision: 0,
    warning: null,
    blocked: false,
  });
  let busy = $state(false);
  let error = $state("");
  // Every load and change takes a number; a reply that a newer one overtook is ignored.
  let epoch = 0;
  // A load asked for while a change ran; it runs once the change ends.
  let refreshPending = false;

  async function load(preservedMutationError = "") {
    const report = hooks.reporter();
    if (!hooks.available()) {
      report("unconfirmed", undefined, "Queue status is unavailable.");
      return false;
    }
    if (busy) {
      refreshPending = true;
      report("unconfirmed", undefined, "Queue update is still in progress.");
      return false;
    }
    report("checking");
    refreshPending = false;
    const mine = ++epoch;
    try {
      const result = await port.list();
      if (
        !Array.isArray(result?.entries) ||
        typeof result.blocked !== "boolean"
      )
        throw new Error("Invalid cleanup queue response");
      if (mine === epoch) {
        view = result;
        error = preservedMutationError;
        report(result.blocked ? "unconfirmed" : "confirmed", result);
        return true;
      }
    } catch (e) {
      if (mine === epoch) {
        error =
          preservedMutationError ||
          `Could not load cleanup queue. ${String(e)}`;
        report("unconfirmed", undefined, error);
      }
    }
    return false;
  }

  // `force`: clearing is the way out of a blocked queue, so it is allowed then.
  async function mutate(
    call: () => Promise<QueueView>,
    force = false,
  ): Promise<boolean> {
    if (!hooks.mutable() || busy || (view.blocked && !force)) return false;
    busy = true;
    error = "";
    const mine = ++epoch;
    const report = hooks.reporter();
    report("checking");
    try {
      const result = await call();
      if (mine !== epoch) return false;
      view = result;
      report(result.blocked ? "unconfirmed" : "confirmed", result);
      hooks.mutated();
      return true;
    } catch (e) {
      error = String(e);
      report("unconfirmed", undefined, error);
      return false;
    } finally {
      busy = false;
      if (refreshPending) void load(error);
    }
  }

  async function remove(entryIds: string[]): Promise<number> {
    const ids = [...new Set(entryIds)];
    if (!ids.length) return 0;
    return (await mutate(() => port.remove(ids))) ? ids.length : 0;
  }

  // Loads once on creation; a reply after teardown is dropped.
  $effect(() => {
    untrack(() => void load());
    return () => {
      ++epoch;
    };
  });

  const ready = $derived(view.entries.filter((e) => e.status === "ready"));
  // A review is addressed to the scan on screen, so it covers only the ready entries the backend matched
  // to that scan (their own root, or a disk scan that covers them).
  const readyHere = $derived.by(() => {
    const scanId = hooks.shownScanId();
    return ready.filter((entry) => !!scanId && entry.matchedScanId === scanId);
  });
  const bytes = $derived(
    view.entries.reduce((sum, entry) => sum + entry.allocatedBytes, 0),
  );
  // One group per disk (or folder root). Each carries its own band and its own review or scan action.
  const groups = $derived.by(() => {
    const hereIds = new Set(readyHere.map((entry) => entry.id));
    const isHere = (entry: QueueEntry) => hereIds.has(entry.id);
    const byRoot = new Map<string, QueueEntry[]>();
    for (const entry of view.entries) {
      const entries = byRoot.get(entry.rootPath) ?? [];
      entries.push(entry);
      byRoot.set(entry.rootPath, entries);
    }
    const sizes = [...byRoot.values()].map((entries) =>
      entries.reduce((sum, entry) => sum + entry.allocatedBytes, 0),
    );
    const largest = Math.max(1, ...sizes);
    return [...byRoot].map(([root, entries], index) => {
      const allocatedBytes = sizes[index];
      // Largest first; 0 B items fall to the end on their own.
      const rows = [...entries].sort(
        (a, b) => b.allocatedBytes - a.allocatedBytes,
      );
      const readyHereRows = rows.filter(isHere);
      // The band is as long as the disk's queued bytes are, relative to the largest disk.
      const share = Math.max(0.06, allocatedBytes / largest);
      const segments = bandSegments(rows, isHere, share, compactBytes);
      // Rows often share a long folder; show it once and list only what differs below it.
      const base = commonFolder(rows.map((entry) => entry.path));
      return {
        base: base === root ? null : base,
        items: folderItems(rows, base),
        root,
        titleId: `cleanup-group-${index}`,
        entries: rows,
        /** The disk's own name, else the root's last component; the view words it. */
        name:
          hooks.diskName(root) || root.split("/").filter(Boolean).at(-1),
        allocatedBytes,
        readyHere: readyHereRows.length,
        // Only ready-elsewhere entries left: opening their scan is enough, so Scan isn't the primary.
        onlyElsewhere: rows.every(
          (entry) => readyHereRows.includes(entry) || entry.status === "ready",
        ),
        // Entries the scan on screen can't review: the disk's scan action is only offered for these.
        unreviewable: readyHereRows.length < rows.length,
        share,
        segments,
        bandSummary:
          allocatedBytes > 0 ? bandSummary(segments, compactBytes) : "",
      };
    });
  });
  const tally = $derived(
    [
      `${view.entries.length} ${view.entries.length === 1 ? "item" : "items"}`,
      `${compactBytes(bytes)} on disk`,
      view.entries.length > ready.length &&
        `${view.entries.length - ready.length} not ready`,
    ]
      .filter(Boolean)
      .join(" · "),
  );

  // The explorer selection measured against the queue, and the add/remove in flight for it.
  const selection = $derived(hooks.selection());
  let addingPaths = $state.raw<string[] | null>(null);
  let removingPaths = $state.raw<string[] | null>(null);
  let removal = $state.raw<{
    paths: string[];
    removed: number;
    attempted: number;
  } | null>(null);
  const marker = $derived(createQueueMarker(view.entries));
  const selectedEntries = $derived.by(() => {
    const paths = new Set(
      selection.map((node) => normalizeQueuePath(queuePathKey(node))),
    );
    return view.entries.filter((entry) => {
      const path = normalizeQueuePath(queuePathKey(entry));
      return path !== null && paths.has(path);
    });
  });
  const directlyQueuedCount = $derived(
    selection.filter((node) =>
      selectedEntries.some(
        (entry) =>
          normalizeQueuePath(queuePathKey(entry)) ===
          normalizeQueuePath(queuePathKey(node)),
      ),
    ).length,
  );
  const queuedCount = $derived(
    selection.filter((node) => marker(node) === "queued").length,
  );
  const queued = $derived(
    selection.length > 0 && queuedCount === selection.length,
  );
  const directlyQueued = $derived(
    selection.length > 0 && directlyQueuedCount === selection.length,
  );
  function matchesSelection(paths: string[] | null) {
    return (
      paths !== null &&
      paths.length === selection.length &&
      selection.every((node) => paths.includes(queuePathKey(node)))
    );
  }
  const adding = $derived(matchesSelection(addingPaths));
  const removing = $derived(matchesSelection(removingPaths));
  const removalFeedback = $derived(
    removal && matchesSelection(removal.paths) ? removal : null,
  );
  // The single-item button already shows its queued state, so only multi-item states and partial failures get a line.
  const feedback = $derived(
    removing
      ? selection.length === 1
        ? ""
        : "Removing from cleanup queue…"
      : removalFeedback
        ? removalFeedback.removed < removalFeedback.attempted
          ? `Removed ${removalFeedback.removed} of ${removalFeedback.attempted} queued items.`
          : removalFeedback.removed === 1
            ? ""
            : `Removed ${removalFeedback.removed} items from cleanup queue.`
        : adding
          ? selection.length === 1
            ? ""
            : "Adding to cleanup queue…"
          : queued
            ? selection.length === 1
              ? ""
              : "All selected items are in your cleanup queue."
            : queuedCount > 0
              ? `${queuedCount} of ${selection.length} selected items already queued.`
              : "",
  );

  return {
    get view() {
      return view;
    },
    get busy() {
      return busy;
    },
    get error() {
      return error;
    },
    get readyHere() {
      return readyHere;
    },
    get groups() {
      return groups;
    },
    get tally() {
      return tally;
    },
    /** Loads the queue again; a change in progress defers it until it ends. */
    refresh: () => load(),
    add: (scanId: string, nodeIds: number[]) =>
      mutate(() => port.add(scanId, nodeIds)),
    /** Removes entries; returns how many, or 0 when the change did not go through. */
    remove,
    clear: () => mutate(() => port.clear(), true),
    /** Asks where an entry must be scanned and hands it to `open`, holding the queue busy meanwhile. */
    async scan(entryId: string, open: (location: Target) => Promise<unknown>) {
      if (busy) return;
      busy = true;
      try {
        await open(await port.target(entryId));
      } catch (e) {
        error = String(e);
      } finally {
        busy = false;
        if (refreshPending) void load();
      }
    },
    prepare: (scanId: string) => port.prepare(scanId),
    /** The explorer selection against the queue. */
    selection: {
      get marker() {
        return marker;
      },
      get entries() {
        return selectedEntries;
      },
      get queuedCount() {
        return queuedCount;
      },
      get inheritedCount() {
        return queuedCount - directlyQueuedCount;
      },
      get queued() {
        return queued;
      },
      get directlyQueued() {
        return directlyQueued;
      },
      get adding() {
        return adding;
      },
      get removing() {
        return removing;
      },
      get removalFeedback() {
        return removalFeedback;
      },
      get feedback() {
        return feedback;
      },
      async add() {
        if (addingPaths || removingPaths) return;
        removal = null;
        addingPaths = selection.map(queuePathKey);
        try {
          await hooks.addSelection();
        } finally {
          addingPaths = null;
        }
      },
      async remove() {
        if (addingPaths || removingPaths || !selectedEntries.length) return;
        const paths = selection.map(queuePathKey);
        const ids = selectedEntries.map((entry) => entry.id);
        removal = null;
        removingPaths = paths;
        try {
          const removed = await remove(ids);
          if (removed > 0) removal = { paths, removed, attempted: ids.length };
        } finally {
          removingPaths = null;
        }
      },
    },
  };
}
