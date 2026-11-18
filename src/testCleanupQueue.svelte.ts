import {
  createCleanupQueue,
  type CleanupQueueHooks,
  type QueuePort,
} from "./cleanupQueue.svelte";
import type { QueueEntry, QueueView } from "./cleanupTypes";
import type { NodeView } from "./types";

/**
 * An in-memory backend queue. `nodes` are what `add` can queue, by id; an added node becomes a
 * ready entry matched to the scan it was added from. Wrap a method in a test to delay or fail it.
 */
export function fakeQueuePort(
  nodes: readonly NodeView[] = [],
  entries: QueueEntry[] = [],
) {
  let view: QueueView = { entries, revision: 0, warning: null, blocked: false };
  const next = (change: Partial<QueueView>) =>
    (view = { ...view, ...change, revision: view.revision + 1 });
  const port: QueuePort = {
    list: async () => view,
    add: async (scanId, nodeIds) =>
      next({
        entries: [
          ...view.entries,
          ...nodes
            .filter((node) => nodeIds.includes(node.id))
            .map((node) => ({
              id: `entry-${node.id}`,
              name: node.name,
              path: node.path,
              rootPath: "/",
              allocatedBytes: node.allocatedBytes,
              status: "ready" as const,
              reason: null,
              matchedScanId: scanId,
            })),
        ],
      }),
    remove: async (ids) =>
      next({ entries: view.entries.filter((entry) => !ids.includes(entry.id)) }),
    clear: async () => next({ entries: [], blocked: false }),
    target: async (entryId) => ({
      id: entryId,
      name: "Root",
      path: "/",
      totalBytes: null,
      freeBytes: null,
    }),
    prepare: async () => ({ token: "token", items: [] }),
  };
  return {
    port,
    /** Replaces the backend's queue without telling the module, as another window would. */
    set: (change: Partial<QueueView>) => next(change),
  };
}

/** createCleanupQueue outside a component: its effect needs a root of its own. */
export function rootedCleanupQueue(port: QueuePort, hooks: CleanupQueueHooks) {
  let queue!: ReturnType<typeof createCleanupQueue>;
  const dispose = $effect.root(() => {
    queue = createCleanupQueue(port, hooks);
  });
  return { queue, dispose };
}

/** The app's side of the hooks as reactive state, so a test changing it reaches the module's derivations. */
export function appState<T extends object>(initial: T): T {
  const state = $state(initial);
  return state;
}
