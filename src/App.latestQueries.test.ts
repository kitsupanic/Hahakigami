// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root, fullEdition } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { NodeView, ScanSummary } from "./types";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  channels: [] as { onmessage: (s: ScanSummary) => void }[],
  app: null as ReturnType<typeof createApp> | null,
}));
vi.mock("./createApp.svelte", async (original) => {
  const module = await original<typeof import("./createApp.svelte")>();
  return { default: () => (mock.app = module.default()) };
});
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  invoke: mock.invoke,
  Channel: class {
    onmessage = (_: ScanSummary) => {};
    constructor() {
      mock.channels.push(this);
    }
  },
}));
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestMap.svelte")).default,
}));
const rootNode: NodeView = {
  id: 0,
  parentId: null,
  name: "Home",
  path: "/fixture",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 1,
  incomplete: false,
};
const file: NodeView = {
  ...rootNode,
  id: 1,
  parentId: 0,
  name: "large.dat",
  path: "/fixture/large.dat",
  kind: "file",
  childrenCount: 0,
};
// A running scan: `entries` changes with every progress event, `elapsedMs` once a second.
const running = (entries: number, elapsedMs = 1000): ScanSummary => ({
  workers: 8,
  workerStats: [],
  finalizationUs: 0,
  scanId: "scan-1",
  rootId: 0,
  state: "running",
  entries,
  allocatedBytes: 100,
  elapsedMs,
  errors: 0,
});
// Calls of one command that answer only when the test resolves them.
let held: Record<string, ((value: unknown) => void)[]>;
let holding: Set<string>;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
let container: HTMLDivElement, root: Root;
async function flush() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}
async function scan() {
  const button = container.querySelector<HTMLButtonElement>(
    "button.welcome-scan",
  )!;
  await act(async () => button.click());
  await flush();
}
const progress = (summary: ScanSummary) =>
  act(async () => {
    mock.channels.at(-1)!.onmessage(summary);
  }).then(flush);
const release = (command: string, index: number, value: unknown) =>
  act(async () => held[command][index](value)).then(flush);

beforeEach(async () => {
  mock.channels.length = 0;
  held = {};
  holding = new Set();
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (holding.has(command))
        return new Promise((resolve) => (held[command] ??= []).push(resolve));
      switch (command) {
        case "get_edition":
          return fullEdition;
        case "get_explorer_preferences":
          return { sort: "sizeDescending", warning: null };
        case "full_disk_access_status":
          return {
            status: "on",
            scanAnyway: false,
            appPath: "/Applications/H.app",
          };
        case "discover_targets":
          return [
            {
              id: "home",
              name: "Home",
              path: "/fixture",
              totalBytes: 1000,
              freeBytes: 500,
            },
          ];
        case "start_scan":
          return "scan-1";
        case "scan_summary":
          return running(2);
        case "get_node":
          return args.nodeId === 1 ? file : rootNode;
        case "get_children":
        case "search_nodes":
          return { items: [file], total: 1 };
        case "get_map":
          return { nodes: [rootNode, file], rootId: 0 };
        default:
          return null;
      }
    },
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(App));
  await flush();
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  localStorage.clear();
});

it("keeps one folder map query in flight across revisions and applies its reply", async () => {
  holding.add("get_map");
  await scan();
  expect(calls("get_map")).toHaveLength(1);
  await progress(running(3, 2000));
  await progress(running(4, 3000));
  expect(calls("get_map")).toHaveLength(1);
  // The reply of the first, now outdated query is dropped; the newest revision runs next, alone.
  await release("get_map", 0, { nodes: [], rootId: 0 });
  expect(calls("get_map")).toHaveLength(2);
  await release("get_map", 1, { nodes: [rootNode, file], rootId: 0 });
  expect(mock.app!.map).toEqual([rootNode, file]);
});

it("sets the root focus although progress events outpace the get_node reply", async () => {
  holding.add("get_node");
  await scan();
  await progress(running(3));
  await progress(running(4));
  expect(mock.app!.focus).toBeNull();
  // The reply started before the newest event, yet its scan is still the one on screen.
  await release("get_node", 0, rootNode);
  expect(mock.app!.focus).toEqual(rootNode);
});

it("refreshes the selected nodes one batch at a time", async () => {
  await scan();
  await act(async () =>
    (container.querySelector("[role=treeitem]") as HTMLButtonElement).click(),
  );
  await flush();
  expect(mock.app!.selection).toHaveLength(1);
  holding.add("get_node");
  const before = calls("get_node").length;
  await progress(running(3, 2000));
  await progress(running(4, 3000));
  await progress(running(5, 4000));
  const selected = () =>
    calls("get_node")
      .slice(before)
      .filter(([, args]) => args.nodeId === 1);
  expect(selected()).toHaveLength(1);
  // Only the newest revision follows the batch in flight.
  await release("get_node", held.get_node.length - 1, file);
  expect(selected()).toHaveLength(2);
});
