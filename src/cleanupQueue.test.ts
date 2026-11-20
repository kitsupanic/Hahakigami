// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { flushSync } from "svelte";
import type { CleanupQueueHooks, QueuePort } from "./cleanupQueue.svelte";
import type { QueueEntry, QueueView } from "./cleanupTypes";
import {
  appState,
  fakeQueuePort,
  rootedCleanupQueue,
} from "./testCleanupQueue.svelte";
import type { NodeView, Target } from "./types";

const node = (id: number, path: string, allocatedBytes = 100): NodeView => ({
  id,
  parentId: 0,
  name: path.split("/").at(-1)!,
  path,
  kind: "file",
  allocatedBytes,
  childrenCount: 0,
  incomplete: false,
});
const entry = (
  id: string,
  path: string,
  change: Partial<QueueEntry> = {},
): QueueEntry => ({
  id,
  name: path.split("/").at(-1)!,
  path,
  rootPath: "/",
  allocatedBytes: 100,
  status: "ready",
  reason: null,
  matchedScanId: "scan-1",
  ...change,
});
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  flushSync();
};

let disposers: (() => void)[] = [];
afterEach(() => {
  disposers.forEach((dispose) => dispose());
  disposers = [];
});

// The app's side, as plain state the tests change.
function setup(
  port: QueuePort,
  app: Partial<{
    mutable: boolean;
    available: boolean;
    selection: NodeView[];
    shownScanId: string | null;
  }> = {},
) {
  const state = appState({
    mutable: true,
    available: true,
    selection: [] as NodeView[],
    shownScanId: "scan-1" as string | null,
    ...app,
  });
  const reports: [string, string | null | undefined][] = [];
  const mutated = vi.fn();
  const hooks: CleanupQueueHooks = {
    available: () => state.available,
    mutable: () => state.mutable,
    reporter: () => (s, _view, error) => reports.push([s, error]),
    mutated,
    selection: () => state.selection,
    shownScanId: () => state.shownScanId,
    diskName: (root) => (root === "/" ? "Macintosh HD" : undefined),
    addSelection: () =>
      queue.add(
        "scan-1",
        state.selection.map((n) => n.id),
      ),
  };
  const { queue, dispose } = rootedCleanupQueue(port, hooks);
  disposers.push(dispose);
  return { queue, state, reports, mutated };
}

it("loads the queue on creation", async () => {
  const { port } = fakeQueuePort([], [entry("a", "/a")]);
  const { queue } = setup(port);
  await settle();
  expect(queue.view.entries.map((e) => e.id)).toEqual(["a"]);
  expect(queue.error).toBe("");
});

it("reports an unavailable queue without asking the backend", async () => {
  const { port } = fakeQueuePort();
  const list = vi.spyOn(port, "list");
  const { queue, reports } = setup(port, { available: false });
  await settle();
  expect(list).not.toHaveBeenCalled();
  expect(await queue.refresh()).toBe(false);
  expect(reports.at(-1)).toEqual(["unconfirmed", "Queue status is unavailable."]);
});

it("rejects a malformed list reply", async () => {
  const { port } = fakeQueuePort();
  port.list = async () => ({ entries: null }) as unknown as QueueView;
  const { queue } = setup(port);
  await settle();
  expect(queue.error).toBe(
    "Could not load cleanup queue. Error: Invalid cleanup queue response",
  );
});

it("ignores a list reply that a change overtook", async () => {
  const fake = fakeQueuePort([node(1, "/a")]);
  const { port } = fake;
  const slow = deferred<QueueView>();
  const { queue, state } = setup(port);
  await settle();
  port.list = () => slow.promise;
  state.selection = [node(1, "/a")];
  const refreshing = queue.refresh();
  await queue.add("scan-1", [1]);
  slow.resolve({ entries: [], revision: 0, warning: null, blocked: false });
  expect(await refreshing).toBe(false);
  expect(queue.view.entries.map((e) => e.id)).toEqual(["entry-1"]);
});

it("defers a refresh asked for during a change until it ends", async () => {
  const { port, set } = fakeQueuePort([node(1, "/a")]);
  const gate = deferred<void>();
  const add = port.add;
  port.add = async (...args) => {
    await gate.promise;
    return add(...args);
  };
  const { queue, reports } = setup(port);
  await settle();
  const adding = queue.add("scan-1", [1]);
  expect(queue.busy).toBe(true);
  expect(await queue.refresh()).toBe(false);
  expect(reports.at(-1)).toEqual([
    "unconfirmed",
    "Queue update is still in progress.",
  ]);
  // Another window changed the queue meanwhile: the deferred load picks it up.
  set({ warning: "changed elsewhere" });
  gate.resolve();
  await adding;
  await settle();
  expect(queue.busy).toBe(false);
  expect(queue.view.warning).toBe("changed elsewhere");
});

it("refuses changes the app or a blocked queue does not allow, except clearing", async () => {
  const { port } = fakeQueuePort([node(1, "/a")], [entry("a", "/a")]);
  const { queue, state, mutated } = setup(port, { mutable: false });
  await settle();
  expect(await queue.add("scan-1", [1])).toBe(false);
  state.mutable = true;
  port.list = async () => ({
    entries: [entry("a", "/a")],
    revision: 1,
    warning: null,
    blocked: true,
  });
  await queue.refresh();
  expect(await queue.remove(["a"])).toBe(0);
  expect(mutated).not.toHaveBeenCalled();
  expect(await queue.clear()).toBe(true);
  expect(queue.view.entries).toEqual([]);
  expect(mutated).toHaveBeenCalledOnce();
});

it("keeps a failed change's message and reports it", async () => {
  const { port } = fakeQueuePort([], [entry("a", "/a")]);
  port.remove = async () => {
    throw "The queue file is locked.";
  };
  const { queue, reports } = setup(port);
  await settle();
  expect(await queue.remove(["a", "a"])).toBe(0);
  expect(queue.error).toBe("The queue file is locked.");
  expect(reports.at(-1)).toEqual(["unconfirmed", "The queue file is locked."]);
});

it("holds only the Scan buttons while an entry's scan starts", async () => {
  const { port } = fakeQueuePort();
  const opened = deferred<void>();
  const open = vi.fn(() => opened.promise);
  const { queue } = setup(port);
  await settle();
  const scanning = queue.scan("a", open);
  await settle();
  expect(open).toHaveBeenCalledWith(expect.objectContaining({ id: "a" }));
  expect(queue.targeting).toBe(true);
  opened.reject("Choose the folder again.");
  await scanning;
  expect(queue.targeting).toBe(false);
  expect(queue.error).toBe("Choose the folder again.");
});

it("leaves the queue free while an entry's target lookup hangs, and ignores a second scan", async () => {
  const { port } = fakeQueuePort([], [entry("a", "/a")]);
  const target = deferred<Target>();
  const scanTarget = vi.spyOn(port, "target").mockReturnValue(target.promise);
  const open = vi.fn(async () => {});
  const { queue } = setup(port);
  await settle();
  const first = queue.scan("a", open);
  await settle();
  expect(queue.targeting).toBe(true);
  expect(queue.busy).toBe(false);
  void queue.scan("a", open);
  await settle();
  expect(scanTarget).toHaveBeenCalledTimes(1);
  // The queue still takes changes meanwhile.
  expect(await queue.remove(["a"])).toBe(1);
  target.resolve({
    id: "a",
    name: "a",
    path: "/a",
    totalBytes: null,
    freeBytes: null,
  });
  await first;
  expect(queue.targeting).toBe(false);
  expect(open).toHaveBeenCalledTimes(1);
});

it("addresses review to the ready entries matched to the scan on screen", async () => {
  const { port } = fakeQueuePort(
    [],
    [
      entry("here", "/a", { allocatedBytes: 300 }),
      entry("elsewhere", "/b", { matchedScanId: "scan-2" }),
      entry("stale", "/c", { status: "changed" }),
      entry("other", "/Volumes/X/d", { rootPath: "/Volumes/X" }),
    ],
  );
  const { queue, state } = setup(port);
  await settle();
  expect(queue.readyHere.map((e) => e.id)).toEqual(["here", "other"]);
  expect(queue.tally).toBe("4 items · 600 B on disk · 1 not ready");
  const [disk, other] = queue.groups;
  expect(disk).toMatchObject({
    root: "/",
    name: "Macintosh HD",
    readyHere: 1,
    unreviewable: true,
    allocatedBytes: 500,
  });
  expect(disk.entries[0].id).toBe("here");
  expect(other).toMatchObject({ name: "X", readyHere: 1, unreviewable: false });
  state.shownScanId = null;
  expect(queue.readyHere).toEqual([]);
});

it("measures the selection against the queue", async () => {
  const folder = { ...node(1, "/a"), kind: "directory" as const };
  const { port } = fakeQueuePort([], [entry("a", "/a")]);
  const { queue, state } = setup(port);
  await settle();
  state.selection = [folder];
  expect(queue.selection.directlyQueued).toBe(true);
  expect(queue.selection.entries.map((e) => e.id)).toEqual(["a"]);

  // A child of a queued folder is queued through it, not directly.
  state.selection = [node(2, "/a/b"), node(3, "/c")];
  expect(queue.selection.queued).toBe(false);
  expect(queue.selection.queuedCount).toBe(1);
  expect(queue.selection.inheritedCount).toBe(1);
  expect(queue.selection.feedback).toBe("1 of 2 selected items already queued.");
});

it("tracks adding and removing the selection, with feedback for several items", async () => {
  const picked = [node(1, "/a"), node(2, "/b")];
  const { port } = fakeQueuePort(picked);
  const gate = deferred<void>();
  const add = port.add;
  port.add = async (...args) => {
    await gate.promise;
    return add(...args);
  };
  const { queue, state } = setup(port, { selection: picked });
  await settle();
  const adding = queue.selection.add();
  flushSync();
  expect(queue.selection.adding).toBe(true);
  expect(queue.selection.feedback).toBe("Adding to cleanup queue…");
  gate.resolve();
  await adding;
  flushSync();
  expect(queue.selection.queued).toBe(true);
  expect(queue.selection.feedback).toBe(
    "All selected items are in your cleanup queue.",
  );

  await queue.selection.remove();
  flushSync();
  expect(queue.view.entries).toEqual([]);
  expect(queue.selection.feedback).toBe("Removed 2 items from cleanup queue.");
  // The feedback belongs to that selection only.
  state.selection = [picked[0]];
  expect(queue.selection.removalFeedback).toBeNull();
});

it("removes unique ids in one request and refuses another change meanwhile", async () => {
  const { port } = fakeQueuePort(
    [],
    [entry("q1", "/q1"), entry("q2", "/q2"), entry("q3", "/q3")],
  );
  const gate = deferred<void>();
  const remove = vi.fn(async (ids: string[]) => {
    await gate.promise;
    return fakeRemove(ids);
  });
  const fakeRemove = port.remove;
  port.remove = remove;
  const { queue } = setup(port);
  await settle();
  const pending = queue.remove(["q1", "q2", "q1"]);
  expect(remove).toHaveBeenCalledWith(["q1", "q2"]);
  expect(await queue.remove(["q3"])).toBe(0);
  gate.resolve();
  expect(await pending).toBe(2);
  expect(remove).toHaveBeenCalledOnce();
  expect(queue.view.entries.map((e) => e.id)).toEqual(["q3"]);
});

it("keeps a failed change's error through the deferred refresh, until a refresh of its own", async () => {
  const { port } = fakeQueuePort([], [entry("q1", "/q1")]);
  const gate = deferred<void>();
  port.remove = async () => {
    await gate.promise;
    throw new Error("Could not persist queue");
  };
  const { queue } = setup(port);
  await settle();
  const list = vi.spyOn(port, "list");
  const pending = queue.remove(["q1"]);
  await queue.refresh();
  expect(list).not.toHaveBeenCalled();
  gate.resolve();
  expect(await pending).toBe(0);
  await settle();
  expect(list).toHaveBeenCalledOnce();
  expect(queue.error).toContain("Could not persist queue");
  expect(queue.view.entries.map((e) => e.id)).toEqual(["q1"]);
  await queue.refresh();
  expect(queue.error).toBe("");
});

it("keeps removing and removal feedback on the selection they were for", async () => {
  const first = node(1, "/a");
  const { port } = fakeQueuePort([], [entry("a", "/a")]);
  const gate = deferred<void>();
  const remove = port.remove;
  port.remove = async (ids) => {
    await gate.promise;
    return remove(ids);
  };
  const { queue, state } = setup(port, { selection: [first] });
  await settle();
  const removing = queue.selection.remove();
  flushSync();
  expect(queue.selection.removing).toBe(true);
  state.selection = [node(2, "/b")];
  expect(queue.selection.removing).toBe(false);
  gate.resolve();
  await removing;
  expect(queue.selection.removalFeedback).toBeNull();
  state.selection = [first];
  expect(queue.selection.removalFeedback).toMatchObject({ removed: 1 });
  // One item's button already says it; no extra line.
  expect(queue.selection.feedback).toBe("");
});

it("matches the selection by exact native path, ignoring only trailing slashes", async () => {
  const { port } = fakeQueuePort(
    [],
    [
      entry("album", "/fixture/album/"),
      { ...entry("ff", "/fixture/native-?"), pathKey: "/fixture/native-%FF" },
    ],
  );
  const { queue, state } = setup(port);
  await settle();
  const pick = (path: string, pathKey?: string) => {
    state.selection = [{ ...node(1, path), pathKey }];
    return queue.selection.entries.map((e) => e.id);
  };
  expect(pick("/fixture/album")).toEqual(["album"]);
  expect(pick("/fixture/Album")).toEqual([]);
  expect(pick("/fixture/album-other")).toEqual([]);
  expect(pick("/fixture/native-?", "/fixture/native-%FF")).toEqual(["ff"]);
  // Same display path, different bytes: not the queued item.
  expect(pick("/fixture/native-?", "/fixture/native-%FE")).toEqual([]);
});
