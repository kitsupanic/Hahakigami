// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type { QueueView } from "./cleanupTypes";
import type { NodeView } from "./types";
const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: mock.invoke,
  Channel: class {
    onmessage = () => {};
  },
}));
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestMap.svelte")).default,
}));
const target = {
  id: "fixture",
  name: "Fixture",
  path: "/fixture",
  totalBytes: null,
  freeBytes: null,
};
const node: NodeView = {
  id: 0,
  parentId: null,
  name: "Fixture",
  path: "/fixture",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 3,
  incomplete: false,
};
const file = (
  id: number,
  name: string,
  kind: NodeView["kind"] = "file",
): NodeView => ({
  ...node,
  id,
  parentId: 0,
  name,
  path: `/fixture/${name}`,
  kind,
  childrenCount: kind === "directory" ? 1 : 0,
});
let items: NodeView[],
  queue: QueueView,
  container: HTMLDivElement,
  root: Root | undefined;
const entries = (...paths: string[]) =>
  paths.map((path, index) => ({
    id: `q${index + 1}`,
    name: path.split("/").at(-1)!,
    path,
    rootPath: "/fixture",
    allocatedBytes: 100,
    status: "needsScan" as const,
    reason: null,
  }));
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
function button(label: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (b) =>
      b.getAttribute("aria-label") === label ||
      b.textContent?.replace(/\s+/g, " ").trim() === label ||
      (label === "Scan location" && b.classList.contains("welcome-scan")),
  );
}
async function click(label: string) {
  const b = button(label);
  expect(b, label).toBeTruthy();
  await act(() => b!.click());
  await flush();
}
function row(id: number) {
  return container.querySelector<HTMLElement>(`.rows [data-node-id="${id}"]`)!;
}
async function select(id: number, additive = false) {
  expect(row(id)).toBeTruthy();
  await act(() =>
    row(id).dispatchEvent(
      new MouseEvent("click", { bubbles: true, ctrlKey: additive }),
    ),
  );
  await flush();
}
const removedIds = () =>
  mock.invoke.mock.calls
    .filter(([c]) => c === "remove_cleanup_queue")
    .flatMap(([, args]) => args.entryIds);
async function mount() {
  root = createTestRoot(container);
  await act(() => root!.render(App));
  await flush();
  await click("Scan location");
}
beforeEach(() => {
  items = [
    file(1, "photo.jpg"),
    file(2, "other.jpg"),
    file(3, "album", "directory"),
  ];
  queue = {
    entries: entries(items[0].path),
    revision: 1,
    warning: null,
    blocked: false,
  };
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command, args) => {
    const summary = {
      scanId: "live",
      state: "completed",
      rootId: 0,
      workers: 1,
      workerStats: [],
      finalizationUs: 0,
      entries: 4,
      allocatedBytes: 100,
      elapsedMs: 10,
      errors: 0,
    };
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
      case "start_scan":
        return "live";
      case "scan_summary":
        return summary;
      case "get_node":
        return node;
      case "get_map":
        return { nodes: items };
      case "get_children":
      case "search_nodes":
        return { items, total: items.length };
      case "list_cleanup_queue":
        return queue;
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
});
afterEach(async () => {
  expect(
    mock.invoke.mock.calls.some(([c]) =>
      /trash|prepare_cleanup_queue|commit_cleanup_queue/.test(c),
    ),
  ).toBe(false);
  await root?.unmount();
  root = undefined;
  container.remove();
});

it("removes a direct entry, updates markers/counts and feedback while preserving selection and scroll", async () => {
  await mount();
  await select(1);
  const viewport = container.querySelector<HTMLElement>(".rows")!;
  viewport.scrollTop = 18;
  expect(row(1).classList.contains("queued")).toBe(true);
  expect(button("Added to cleanup queue. Remove from cleanup queue")?.disabled).toBe(false);
  await click("Added to cleanup queue. Remove from cleanup queue");
  expect(removedIds()).toEqual(["q1"]);
  expect(row(1).classList.contains("queued")).toBe(false);
  expect(row(1).getAttribute("aria-selected")).toBe("true");
  expect(viewport.scrollTop).toBe(18);
  expect(button("Add to cleanup queue")?.disabled).toBe(false);
  expect(container.querySelector<HTMLElement>(".queue-feedback")?.hidden).toBe(
    true,
  );
  expect(container.querySelector(".queue-strip")).toBeNull();
  expect(container.querySelector(".queue-count")?.textContent).toBe("0");
});

it("keeps failed removal retryable without announcing success", async () => {
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "remove_cleanup_queue"
      ? Promise.reject(new Error("Queue write failed"))
      : standard(command, args),
  );
  await mount();
  await select(1);
  await click("Added to cleanup queue. Remove from cleanup queue");
  expect(
    [...container.querySelectorAll('[role="alert"]')]
      .map((alert) => alert.textContent)
      .join(" "),
  ).toContain("Queue write failed");
  expect(button("Added to cleanup queue. Remove from cleanup queue")?.disabled).toBe(false);
  expect(container.querySelector(".queue-feedback")?.textContent).not.toContain(
    "Removed",
  );
  expect(row(1).classList.contains("queued")).toBe(true);
  mock.invoke.mockImplementation(standard);
  await click("Added to cleanup queue. Remove from cleanup queue");
  expect(container.querySelector<HTMLElement>(".queue-feedback")?.hidden).toBe(
    true,
  );
});

it("removes all directly queued selections by ID", async () => {
  queue.entries = entries(items[0].path, items[1].path);
  await mount();
  await select(1);
  await select(2, true);
  expect(button("Added to cleanup queue. Remove selected items from queue")?.disabled).toBe(false);
  await click("Added to cleanup queue. Remove selected items from queue");
  expect(removedIds()).toEqual(["q1", "q2"]);
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "remove_cleanup_queue",
    ),
  ).toEqual([["remove_cleanup_queue", { entryIds: ["q1", "q2"] }]]);
  expect(row(1).getAttribute("aria-selected")).toBe("true");
  expect(row(2).getAttribute("aria-selected")).toBe("true");
  expect(container.querySelector(".queue-feedback")?.textContent).toContain(
    "Removed 2 items from cleanup queue.",
  );
});

it("offers add remaining plus exact-only removal for mixed selections", async () => {
  await mount();
  await select(1);
  await select(2, true);
  expect(button("Add remaining to cleanup queue")?.disabled).toBe(false);
  expect(button("Remove 1 queued item")?.disabled).toBe(false);
  await click("Remove 1 queued item");
  expect(removedIds()).toEqual(["q1"]);
  expect(button("Add to cleanup queue")).toBeTruthy();
});

it("explains inherited membership and reviews the queue without removing a parent", async () => {
  queue.entries = entries("/fixture");
  await mount();
  await select(1);
  expect(
    container.querySelector(".queue-inherited-feedback")?.textContent,
  ).toContain("included through a queued parent folder");
  expect(button("Added to cleanup queue. Remove from cleanup queue")).toBeUndefined();
  expect(container.querySelector(".queue-remove-partial")).toBeNull();
  await act(() =>
    container.querySelector<HTMLButtonElement>(".queue-review")!.click(),
  );
  await flush();
  expect(
    container.querySelector(".cleanup-entry .cleanup-figure"),
  ).not.toBeNull();
  expect(removedIds()).toEqual([]);
});

it("removes only the exact entry in mixed exact and inherited selections", async () => {
  items[1] = file(2, "album/inside.jpg");
  queue.entries = entries(items[0].path, "/fixture/album");
  await mount();
  await select(1);
  await select(2, true);
  expect(
    container.querySelector(".queue-inherited-feedback")?.textContent,
  ).toContain("1 selected item is included");
  expect(container.querySelector(".queue-review")).toBeTruthy();
  await click("Remove 1 queued item");
  expect(removedIds()).toEqual(["q1"]);
  expect(queue.entries.map((entry) => entry.path)).toEqual(["/fixture/album"]);
});

it("keeps Add available on a folder containing queued descendants", async () => {
  queue.entries = entries("/fixture/album/inside.jpg");
  await mount();
  await select(3);
  expect(button("Add to cleanup queue")?.disabled).toBe(false);
  expect(button("Added to cleanup queue. Remove from cleanup queue")).toBeUndefined();
  expect(removedIds()).toEqual([]);
});

it("does not match a path to a queue entry with a different native key", async () => {
  items = [{ ...file(2, "native-�"), pathKey: "/fixture/native-%FF" }];
  queue.entries = [
    {
      ...entries(items[0].path)[0],
      pathKey: "/fixture/native-%EF%BF%BD",
    },
  ];
  await mount();
  await vi.waitFor(() => expect(row(2)).toBeTruthy());
  await select(2);
  expect(row(2).classList.contains("queued")).toBe(false);
  expect(button("Added to cleanup queue. Remove from cleanup queue")).toBeUndefined();
  expect(removedIds()).toEqual([]);
});
