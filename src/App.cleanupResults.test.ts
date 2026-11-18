// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import Harness from "./QueueRemovalHarness.svelte";
import type createApp from "./createApp.svelte";
import type { QueueView } from "./cleanupTypes";
import type { NodeView, ScanSummary, Target } from "./types";
import { cleanupResult } from "./cleanupResults";

const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: mock.invoke,
  Channel: class {
    onmessage = () => {};
  },
}));
let app: ReturnType<typeof createApp>, root: Root, container: HTMLDivElement;
let queue: QueueView;
const target: Target = {
  id: "home",
  name: "Home",
  path: "/fixture",
  totalBytes: null,
  freeBytes: null,
};
const summary: ScanSummary = {
  scanId: "live",
  rootId: 0,
  state: "completed",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 3,
  logicalBytes: 200,
  allocatedBytes: 200,
  elapsedMs: 10,
  errors: 0,
};
const items: NodeView[] = [1, 2].map((id) => ({
  id,
  parentId: 0,
  name: "same",
  path: "/fixture/same",
  kind: "file",
  logicalBytes: 100,
  allocatedBytes: 100,
  childrenCount: 0,
  incomplete: false,
}));
const rootNode: NodeView = {
  ...items[0],
  id: 0,
  parentId: null,
  name: "Home",
  path: "/fixture",
  kind: "directory",
};
const partial = () => ({
  trashed: 1,
  failures: ["same: permission denied"],
  outcomes: [
    { reviewIndex: 1, status: "failed", message: "Native permission denied" },
    { reviewIndex: 0, status: "trashed" },
  ],
});
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
function review(queued = false) {
  app.setSummary(summary);
  app.setTrash({ token: "ticket", items, queued });
}
beforeEach(async () => {
  queue = {
    entries: items.map((item) => ({
      id: `q${item.id}`,
      name: item.name,
      path: item.path,
      rootPath: "/fixture",
      logicalBytes: 100,
      allocatedBytes: 100,
      status: "ready",
      reason: null,
    })),
    revision: 1,
    blocked: false,
    warning: null,
  };
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command, args) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return {
            status: "on",
            scanAnyway: false,
            appPath: "/Fixture.app",
        };
      case "discover_targets":
        return [target];
      case "list_cleanup_queue":
        return queue;
      case "start_scan":
        return "live";
      case "scan_summary":
        return summary;
      case "get_node":
        return rootNode;
      case "get_map":
        return { nodes: items };
      case "get_children":
        return { items, total: 2 };
      case "prepare_trash":
      case "prepare_cleanup_queue":
        return { token: "ticket", items };
      case "commit_trash":
      case "commit_cleanup_queue":
        return partial();
      case "remove_cleanup_queue":
        queue = {
          ...queue,
          entries: queue.entries.filter(
            (entry) => !args.entryIds.includes(entry.id),
          ),
          revision: queue.revision + 1,
        };
        return queue;
      default:
        return undefined;
    }
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() =>
    root.render(Harness, {
      capture: (value) => {
        app = value;
      },
    }),
  );
  await flush();
  await act(() => app.start(target));
  await flush();
});
afterEach(async () => {
  await root.unmount();
  container.remove();
});

it("maps exact reviewed indices despite duplicate paths and preserves native failures separately", async () => {
  review(true);
  await act(() => app.commitTrash());
  await flush();
  expect(app.cleanupReport).toMatchObject({
    source: "queue",
    state: "confirmed",
    reviewedCount: 2,
    movedCount: 1,
    failedCount: 1,
    detailsAvailable: true,
    remainingQueued: 2,
    queueState: "confirmed",
  });
  expect(app.cleanupReport?.movedItems.map((item) => item.id)).toEqual([1]);
  expect(
    app.cleanupReport?.failedItems.map(({ item, message }) => [
      item.id,
      message,
    ]),
  ).toEqual([[2, "Native permission denied"]]);
  expect(app.cleanupReport?.failures).toEqual(["same: permission denied"]);
  expect(app.error).toBe("");
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "start_scan"),
  ).toHaveLength(2);
});
it.each([0, 2])(
  "confirms %i moves and only rescans when items moved",
  async (trashed) => {
    const standard = mock.invoke.getMockImplementation()!;
    mock.invoke.mockImplementation((command, args) =>
      command === "commit_trash"
        ? Promise.resolve({
            trashed,
            failures: trashed ? [] : ["one", "two"],
            outcomes: items.map((_, reviewIndex) => ({
              reviewIndex,
              status: trashed ? "trashed" : "failed",
              message: trashed ? null : "Denied",
            })),
          })
        : standard(command, args),
    );
    review();
    await act(() => app.commitTrash());
    await flush();
    expect(app.cleanupReport).toMatchObject({
      state: "confirmed",
      movedCount: trashed,
      failedCount: 2 - trashed,
      detailsAvailable: true,
    });
    expect(
      mock.invoke.mock.calls.filter(([command]) => command === "start_scan"),
    ).toHaveLength(trashed ? 2 : 1);
  },
);
it("freezes reviewed item metadata and prevents duplicate commits while pending", async () => {
  const commit = deferred<ReturnType<typeof partial>>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "commit_trash" ? commit.promise : standard(command, args),
  );
  const localItems = items.map((item) => ({ ...item }));
  app.setTrash({ token: "ticket", items: localItems });
  const pending = app.commitTrash();
  localItems[0].name = "changed after activation";
  await app.commitTrash();
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "commit_trash"),
  ).toHaveLength(1);
  commit.resolve(partial());
  await act(() => pending);
  expect(app.cleanupReport?.movedItems[0].name).toBe("same");
});
it("keeps the receipt through a rescan failure, browsing and the next cancelled review", async () => {
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "start_scan"
      ? Promise.reject("Rescan unavailable")
      : standard(command, args),
  );
  review();
  await act(() => app.commitTrash());
  await flush();
  const id = app.cleanupReport!.id;
  expect(app.cleanupReport?.movedCount).toBe(1);
  expect(app.error).toContain("Rescan unavailable");
  app.showQueue(true);
  app.showQueue(false);
  app.setSummary(summary);
  app.select(items[0]);
  await act(() => app.prepareTrash());
  app.setTrash(null);
  expect(app.cleanupReport?.id).toBe(id);
  expect(app.cleanupReport?.movedCount).toBe(1);
});
it("keeps confirmed cleanup outcomes visible while the automatic rescan is still starting", async () => {
  const scan = deferred<string>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "start_scan" ? scan.promise : standard(command, args),
  );
  review();
  const pending = app.commitTrash();
  await flush();
  expect(app.busy).toBe(true);
  expect(app.summary).toBeNull();
  expect(app.cleanupReport).toMatchObject({
    state: "confirmed",
    movedCount: 1,
    failedCount: 1,
  });
  scan.resolve("live");
  await act(() => pending);
  expect(app.cleanupReport?.movedCount).toBe(1);
});
it("shows checking while a post-cleanup refresh is pending, then confirms the actual queue without subtraction", async () => {
  const list = deferred<QueueView>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "list_cleanup_queue"
      ? list.promise
      : command === "commit_trash"
        ? Promise.resolve({
            ...partial(),
            persistenceWarning: "Could not persist removals",
          })
        : standard(command, args),
  );
  review();
  const pending = app.commitTrash();
  await flush();
  expect(app.cleanupReport).toMatchObject({
    state: "confirmed",
    queueState: "checking",
    remainingQueued: null,
    persistenceWarning: "Could not persist removals",
  });
  list.resolve(queue);
  await act(() => pending);
  await flush();
  expect(app.cleanupReport?.remainingQueued).toBe(2);
  expect(app.cleanupReport?.persistenceWarning).toBe(
    "Could not persist removals",
  );
  expect(app.error).toBe("");
});
it.each(["failure", "blocked"])(
  "does not confirm stale queue counts after %s and permits refresh retry",
  async (mode) => {
    const standard = mock.invoke.getMockImplementation()!;
    mock.invoke.mockImplementation((command, args) =>
      command === "list_cleanup_queue"
        ? mode === "failure"
          ? Promise.reject("Queue storage unavailable")
          : Promise.resolve({
              ...queue,
              blocked: true,
              warning: "Queue needs repair",
            })
        : standard(command, args),
    );
    review();
    await act(() => app.commitTrash());
    await flush();
    expect(app.cleanupReport).toMatchObject({
      movedCount: 1,
      queueState: "unconfirmed",
      remainingQueued: null,
    });
    expect(
      mode === "failure"
        ? app.cleanupReport?.queueError
        : app.cleanupReport?.queueWarning,
    ).toContain(
      mode === "failure" ? "Queue storage unavailable" : "Queue needs repair",
    );
    mock.invoke.mockImplementation(standard);
    await act(() => app.refreshQueue());
    expect(app.cleanupReport).toMatchObject({
      queueState: "confirmed",
      remainingQueued: 2,
      queueError: null,
    });
  },
);
it("marks a thrown command as unknown and never assumes all items failed or retries Trash", async () => {
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "commit_trash"
      ? Promise.reject("Connection lost after command")
      : standard(command, args),
  );
  review();
  await act(() => app.commitTrash());
  expect(app.cleanupReport).toMatchObject({
    state: "unconfirmed",
    movedCount: null,
    failedCount: null,
    detailsAvailable: false,
    queueState: "confirmed",
  });
  expect(app.cleanupReport?.message).toContain("Connection lost after command");
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "start_scan"),
  ).toHaveLength(1);
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "commit_trash"),
  ).toHaveLength(1);
});
it("does not resurrect a dismissed receipt or update a newer receipt from old queue responses", async () => {
  review();
  await act(() => app.commitTrash());
  await flush();
  const old = deferred<QueueView>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "list_cleanup_queue" ? old.promise : standard(command, args),
  );
  const pending = app.refreshQueue();
  app.dismissCleanupReport();
  old.resolve({ ...queue, entries: [] });
  await act(() => pending);
  expect(app.cleanupReport).toBeNull();
  mock.invoke.mockImplementation(standard);
  review();
  await act(() => app.commitTrash());
  await flush();
  const stale = deferred<QueueView>();
  let calls = 0;
  mock.invoke.mockImplementation((command, args) =>
    command === "list_cleanup_queue" && ++calls === 1
      ? stale.promise
      : standard(command, args),
  );
  const staleLoad = app.refreshQueue();
  review();
  await act(() => app.commitTrash());
  await flush();
  const id = app.cleanupReport!.id;
  stale.resolve({ ...queue, entries: [] });
  await act(() => staleLoad);
  expect(app.cleanupReport).toMatchObject({ id, remainingQueued: 2 });
});
it("updates receipt counts after accepted queue mutations and keeps deferred refresh semantics", async () => {
  review();
  await act(() => app.commitTrash());
  await flush();
  const mutation = deferred<QueueView>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "remove_cleanup_queue"
      ? mutation.promise
      : standard(command, args),
  );
  const pending = app.removeQueue("q1");
  await app.refreshQueue();
  expect(app.cleanupReport).toMatchObject({
    queueState: "unconfirmed",
    remainingQueued: null,
  });
  queue = { ...queue, entries: queue.entries.slice(1) };
  mutation.resolve(queue);
  await act(() => pending);
  await flush();
  expect(app.cleanupReport).toMatchObject({
    movedCount: 1,
    queueState: "confirmed",
    remainingQueued: 1,
  });
});
it.each([
  undefined,
  [
    { reviewIndex: 0, status: "trashed" },
    { reviewIndex: 0, status: "failed" },
  ],
  [
    { reviewIndex: -1, status: "trashed" },
    { reviewIndex: 1, status: "failed" },
  ],
  [
    { reviewIndex: 0.5, status: "trashed" },
    { reviewIndex: 1, status: "failed" },
  ],
  [{ reviewIndex: 0, status: "trashed" }],
  [
    { reviewIndex: 0, status: "trashed" },
    { reviewIndex: 1, status: "trashed" },
  ],
  [
    { reviewIndex: 0, status: "unknown" },
    { reviewIndex: 1, status: "failed" },
  ],
])(
  "retains confirmed aggregates without fabricating item identities for outcomes %j",
  (outcomes) => {
    expect(
      cleanupResult(1, "selection", items, { ...partial(), outcomes }),
    ).toMatchObject({
      state: "confirmed",
      movedCount: 1,
      failedCount: 1,
      detailsAvailable: false,
      movedItems: [],
      failedItems: [],
      failures: ["same: permission denied"],
    });
  },
);
it.each([
  { trashed: 2, failures: ["inconsistent"] },
  { trashed: -1, failures: [] },
  { trashed: Infinity, failures: [] },
  { trashed: 0.5, failures: [] },
])("keeps malformed aggregates unconfirmed for %j", (value) => {
  expect(cleanupResult(1, "selection", items, value)).toMatchObject({
    state: "unconfirmed",
    movedCount: null,
    failedCount: null,
    detailsAvailable: false,
  });
});
