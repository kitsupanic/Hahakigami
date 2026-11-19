// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import Harness from "./QueueRemovalHarness.svelte";
import type createApp from "./createApp.svelte";
import type { QueueView } from "./cleanupTypes";

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
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function removed(ids: string[]): QueueView {
  queue = {
    ...queue,
    revision: queue.revision + 1,
    entries: queue.entries.filter((entry) => !ids.includes(entry.id)),
  };
  return queue;
}
const removals = () =>
  mock.invoke.mock.calls.filter(
    ([command]) => command === "remove_cleanup_queue",
  );
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
beforeEach(async () => {
  queue = {
    entries: ["q1", "q2", "q3"].map((id) => ({
      id,
      name: id,
      path: `/fixture/${id}`,
      rootPath: "/fixture",
      allocatedBytes: 100,
      status: "needsScan",
      reason: null,
    })),
    revision: 1,
    blocked: false,
    warning: null,
  };
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command, args) => {
    switch (command) {
      case "list_cleanup_queue":
        return queue;
      case "remove_cleanup_queue":
        return removed(args.entryIds);
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "discover_targets":
        return [];
      case "full_disk_access_status":
        return { status: "off", scanAnyway: false, appPath: "/Fixture.app" };
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
});
afterEach(async () => {
  await root.unmount();
  container.remove();
});

it("removes stale saved entries without access or a fresh scan and preserves single removal compatibility", async () => {
  expect(app.summary).toBeNull();
  expect(app.access?.status).toBe("off");
  expect(await app.removeQueue("q1")).toBe(1);
  expect(app.queue.entries.map((entry) => entry.id)).toEqual(["q2", "q3"]);
  expect(
    mock.invoke.mock.calls.some(([command]) =>
      /trash|prepare_cleanup_queue|commit_cleanup_queue/.test(command),
    ),
  ).toBe(false);
});

it("guards empty batches, other operations and blocked storage", async () => {
  expect(await app.removeQueues([])).toBe(0);
  app.setBusy(true);
  expect(await app.removeQueues(["q1"])).toBe(0);
  app.setBusy(false);
  queue = { ...queue, blocked: true };
  await app.refreshQueue();
  expect(await app.removeQueues(["q1"])).toBe(0);
  expect(removals()).toHaveLength(0);
});

it("invalidates a queued Trash ticket and drains a refresh only after the whole batch", async () => {
  const batch = deferred<QueueView>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "remove_cleanup_queue"
      ? batch.promise
      : standard(command, args),
  );
  app.setTrash({
    queued: true,
    scanId: "live",
    token: "old-ticket",
    items: [],
  });
  const pending = app.removeQueues(["q1", "q2"]);
  const listCount = mock.invoke.mock.calls.filter(
    ([command]) => command === "list_cleanup_queue",
  ).length;
  await app.refreshQueue();
  await flush();
  expect(app.trash?.token).toBe("old-ticket");
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "list_cleanup_queue",
    ),
  ).toHaveLength(listCount);
  batch.resolve(removed(["q1", "q2"]));
  expect(await pending).toBe(2);
  await flush();
  expect(app.trash).toBeNull();
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "list_cleanup_queue",
    ),
  ).toHaveLength(listCount + 1);
  expect(app.queue.entries.map((entry) => entry.id)).toEqual(["q3"]);
});
