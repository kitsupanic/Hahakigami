// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setShowLogicalSize } from "./logicalSize.svelte";
import type createApp from "./createApp.svelte";
import type { NodeView, ScanSummary } from "./types";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  emit: vi.fn(),
  listen: vi.fn(),
  listeners: new Map<
    string,
    (event: { payload: { scanId: string } }) => void
  >(),
  order: [] as string[],
  lastPaint: null as (() => void) | null,
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
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  emit: mock.emit,
  listen: mock.listen,
}));
// The button models a completed paint of the current committed map props. Preparing the
// map alone must not report rendering, and an old map must not report a new scan's result.
vi.mock("./DiskMap.svelte", async () => {
  const { performanceMapRecord } = await import("./testUtils");
  performanceMapRecord.current = mock;
  return { default: (await import("./TestPerformanceMap.svelte")).default };
});

let container: HTMLDivElement;
let root: Root;
let starts: number;
const node = (scanId: string): NodeView => ({
  id: 0,
  parentId: null,
  name: scanId,
  path: "/fixture",
  kind: "directory",
  logicalBytes: 100,
  allocatedBytes: 100,
  childrenCount: 0,
  incomplete: false,
});
const summary = (scanId: string): ScanSummary => ({
  scanId,
  rootId: 0,
  state: "completed",
  scanPhase: "done",
  workers: 8,
  workerStats: [],
  finalizationUs: 0,
  entries: 1,
  logicalBytes: 100,
  allocatedBytes: 100,
  elapsedMs: 10,
  errors: 0,
});
async function flush() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}
async function mount(mode: string) {
  window.__PERFORMANCE_BENCHMARK__ = { root: "/fixture", mode };
  const { default: App } = await import("./App.svelte");
  await act(async () => root.render(App));
  await flush();
  await flush();
}
async function paint() {
  const button = Array.from(container.querySelectorAll("button")).find(
    (item) => item.textContent === "Paint current map",
  );
  expect(button).toBeTruthy();
  await act(async () => {
    button!.click();
  });
  await flush();
}
async function ready(...scanIds: string[]) {
  const listener = mock.listeners.get("performance-rescan-ready");
  expect(listener).toBeTruthy();
  await act(async () => {
    for (const scanId of scanIds) listener!({ payload: { scanId } });
  });
  await flush();
  await flush();
}
function rendered() {
  return mock.emit.mock.calls
    .filter(([name]) => name === "performance-rendered")
    .map(([, payload]) => payload.scanId);
}

beforeEach(() => {
  setShowLogicalSize(true);
  vi.stubEnv("VITE_PERFORMANCE_BENCHMARK", "1");
  mock.listeners.clear();
  mock.order.length = 0;
  mock.lastPaint = null;
  mock.invoke.mockReset();
  mock.emit.mockReset();
  mock.listen.mockReset();
  mock.listen.mockImplementation(
    async (
      name: string,
      callback: (event: { payload: { scanId: string } }) => void,
    ) => {
      mock.listeners.set(name, callback);
      return () => mock.listeners.delete(name);
    },
  );
  starts = 0;
  mock.emit.mockImplementation(
    async (name: string, payload?: { scanId: string }) => {
      mock.order.push(
        name === "performance-rendered" ? `render:${payload?.scanId}` : name,
      );
    },
  );
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown> = {}) => {
      switch (command) {
        case "get_explorer_preferences":
          return { sort: "sizeDescending", warning: null };
        case "full_disk_access_status":
          return {
            status: "on",
            scanAnyway: false,
            appPath: "/fixture/app",
          };
        case "discover_targets":
          return [
            {
              id: "fixture",
              name: "Fixture",
              path: "/fixture",
              totalBytes: 1000,
              freeBytes: 500,
            },
            {
              id: "other",
              name: "Other",
              path: "/other",
              totalBytes: 1000,
              freeBytes: 500,
            },
          ];
        case "start_scan": {
          const scanId = `scan-${++starts}`;
          mock.order.push(`scan:${scanId}`);
          return scanId;
        }
        case "scan_summary":
          return summary(args.scanId as string);
        case "get_node":
          return node(args.scanId as string);
        case "get_map":
          return { nodes: [node(args.scanId as string)], rootId: 0 };
        case "get_children":
          return { items: [], total: 0 };
        default:
          return null;
      }
    },
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  delete window.__PERFORMANCE_BENCHMARK__;
  vi.unstubAllEnvs();
});

it.each(["same_process", "changed"])(
  "%s paints the preparation scan and starts one real UI rescan",
  async (mode) => {
    await mount(mode);
    expect(starts).toBe(1);
    expect(rendered()).toEqual([]);
    // A seed that has not painted, and an unrelated scan, cannot authorize a rescan.
    await ready("scan-1", "unrelated");
    expect(starts).toBe(1);
    await paint();
    expect(rendered()).toEqual(["scan-1"]);
    // Changing the selected target must not redirect the benchmark's original scan authority.
    const other = mock.app!.targets.find((item) => item.name === "Other");
    expect(other).toBeTruthy();
    await act(async () => mock.app!.setTarget(other!));
    await ready("unrelated", "scan-1", "scan-1");
    expect(starts).toBe(2);
    expect(
      mock.invoke.mock.calls
        .filter(([command]) => command === "start_scan")
        .map(([, args]) => args.targetId),
    ).toEqual(["fixture", "fixture"]);
    expect(rendered()).toEqual(["scan-1"]);
    await ready("scan-1", "scan-2");
    expect(starts).toBe(2);
    await paint();
    expect(rendered()).toEqual(["scan-1", "scan-2"]);
    await paint();
    await ready("scan-1", "scan-2", "scan-2");
    expect(starts).toBe(2);
    expect(rendered()).toEqual(["scan-1", "scan-2"]);
    expect(mock.order).toEqual([
      "performance-start",
      "scan:scan-1",
      "draw:scan-1",
      "render:scan-1",
      "performance-start",
      "scan:scan-2",
      "draw:scan-2",
      "render:scan-2",
      "draw:scan-2",
    ]);
  },
);

it("full mode completes with one UI scan and one committed-map render signal", async () => {
  await mount("full");
  expect(starts).toBe(1);
  expect(rendered()).toEqual([]);
  await paint();
  await paint();
  await ready("scan-1", "scan-1");
  expect(starts).toBe(1);
  expect(rendered()).toEqual(["scan-1"]);
});

it("waits for asynchronous rescan event registration before beginning its seed scan", async () => {
  let register: (() => void) | undefined;
  mock.listen.mockImplementation(
    (
      name: string,
      callback: (event: { payload: { scanId: string } }) => void,
    ) => {
      if (name !== "performance-rescan-ready") {
        mock.listeners.set(name, callback);
        return Promise.resolve(() => mock.listeners.delete(name));
      }
      return new Promise<() => void>((resolve) => {
        register = () => {
          mock.listeners.set(name, callback);
          resolve(() => mock.listeners.delete(name));
        };
      });
    },
  );
  await mount("same_process");
  expect(register).toBeTruthy();
  expect(starts).toBe(0);
  expect(mock.emit).not.toHaveBeenCalled();
  await act(async () => {
    register!();
  });
  await flush();
  await flush();
  expect(starts).toBe(1);
  await paint();
  await ready("scan-1");
  expect(starts).toBe(2);
});

it("fences seed channel events before awaiting the second start notification", async () => {
  await mount("same_process");
  await paint();
  const paintSeed = mock.lastPaint!;
  const seedChannel = mock.invoke.mock.calls.find(
    ([command]) => command === "start_scan",
  )![1].onEvent as {
    onmessage: (value: ScanSummary) => void;
  };
  let releaseStart: (() => void) | undefined;
  mock.emit.mockImplementation((name: string, payload?: { scanId: string }) => {
    mock.order.push(
      name === "performance-rendered" ? `render:${payload?.scanId}` : name,
    );
    if (name === "performance-start")
      return new Promise<void>((resolve) => {
        releaseStart = resolve;
      });
    return Promise.resolve();
  });
  await ready("scan-1");
  expect(releaseStart).toBeTruthy();
  expect(starts).toBe(1);
  await act(async () => {
    seedChannel.onmessage({
      ...summary("scan-1"),
      state: "failed",
      message: "STALE SEED FAILURE",
    });
  });
  expect(container.textContent).not.toContain("STALE SEED FAILURE");
  // The still-committed seed map cannot finish the pending second scan.
  await act(async () => {
    paintSeed();
  });
  expect(rendered()).toEqual(["scan-1"]);
  await ready("scan-1", "scan-1");
  expect(starts).toBe(1);
  await act(async () => {
    releaseStart!();
  });
  await flush();
  await flush();
  expect(starts).toBe(2);
  expect(rendered()).toEqual(["scan-1"]);
  await paint();
  expect(rendered()).toEqual(["scan-1", "scan-2"]);
  await ready("scan-1", "scan-2");
  expect(starts).toBe(2);
});

it("keeps map readiness false through pending metric queries, stale replies and old draws", async () => {
  await mount("full");
  await paint();
  const nativeRoot = container.querySelector<HTMLElement>(".app")!;
  expect(nativeRoot.dataset.nativeMapReady).toBe("true");
  const paintOriginal = mock.lastPaint!;
  const original = mock.invoke.getMockImplementation()!;
  const pending: {
    metric: string;
    resolve: (value: { nodes: NodeView[] }) => void;
  }[] = [];
  mock.invoke.mockImplementation((command, args) =>
    command === "get_map"
      ? new Promise((resolve) => pending.push({ metric: args.metric, resolve }))
      : original(command, args),
  );
  async function metric(label: string) {
    const button = [...container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label,
    )!;
    expect(button).toBeTruthy();
    await act(async () => button.click());
    await flush();
  }

  await metric("Logical");
  expect(pending.map((request) => request.metric)).toEqual(["logical"]);
  expect(nativeRoot.dataset.nativeMapReady).toBe("false");
  await act(async () => paintOriginal());
  expect(nativeRoot.dataset.nativeMapReady).toBe("false");

  // A second request supersedes the first without allowing its reply to clear pending.
  await metric("On disk");
  expect(pending.map((request) => request.metric)).toEqual([
    "logical",
    "allocated",
  ]);
  await act(async () => pending[0].resolve({ nodes: [node("stale-logical")] }));
  await flush();
  await act(async () => paintOriginal());
  expect(nativeRoot.dataset.nativeMapReady).toBe("false");
  expect(rendered()).toEqual(["scan-1"]);

  await act(async () =>
    pending[1].resolve({ nodes: [node("current-allocated")] }),
  );
  await flush();
  expect(nativeRoot.dataset.nativeMapReady).toBe("false");
  // The committed dataset changed, so the previous dataset's callback is still obsolete.
  await act(async () => paintOriginal());
  expect(nativeRoot.dataset.nativeMapReady).toBe("false");
  await paint();
  expect(nativeRoot.dataset.nativeMapReady).toBe("true");
  expect(mock.order.at(-1)).toBe("draw:current-allocated");
  expect(rendered()).toEqual(["scan-1"]);
});
