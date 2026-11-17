// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import { menuItem } from "./fakeMenu";
import type createApp from "./createApp.svelte";
import type { NodeView, SavedEntry, ScanSummary, Target } from "./types";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
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
    onmessage = (_: unknown) => {};
  },
}));
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestMap.svelte")).default,
}));
const alpha: Target = {
  id: "alpha",
  name: "Projects",
  path: "/alpha/Projects",
  totalBytes: null,
  freeBytes: null,
};
// Same display name deliberately exercises source identity rather than labels.
const beta: Target = { ...alpha, id: "beta", path: "/beta/Projects" };
const summary = (
  scanId: string,
  origin: "saved" | "live" = "live",
): ScanSummary => ({
  scanId,
  origin,
  rootId: 0,
  state: "completed",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 2,
  logicalBytes: 200,
  allocatedBytes: 100,
  elapsedMs: 1000,
  errors: 0,
  ...(origin === "saved" ? { savedId: "saved-alpha" } : {}),
});
const saved: SavedEntry = {
  id: "saved-alpha",
  name: alpha.name,
  rootPath: alpha.path,
  startedAtMs: 1000,
  completedAtMs: 2000,
  savedAtMs: 3000,
  entries: 2,
  logicalBytes: 200,
  allocatedBytes: 100,
  incomplete: false,
  errors: 0,
  excluded: 0,
  fileBytes: 50,
};
const node = (target: Target, child = false): NodeView => ({
  id: child ? 1 : 0,
  parentId: child ? 0 : null,
  name: child ? "archive.zip" : target.name,
  path: target.path + (child ? "/archive.zip" : ""),
  kind: child ? "file" : "directory",
  logicalBytes: 200,
  allocatedBytes: 100,
  childrenCount: child ? 0 : 1,
  incomplete: false,
});
let container: HTMLDivElement, root: Root, startupSaved: boolean;
const app = () => mock.app!;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
const button = (label: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) =>
      item.getAttribute("aria-label") === label ||
      item.textContent?.replace(/\s+/g, " ").trim() === label,
  )!;
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve(); // startup asks saved_scans_enabled before list_saved_scans
  });
}
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
async function mount() {
  await act(() => root.render(App));
  await settle();
}
const rescanAction = () =>
  container.querySelector<HTMLButtonElement>("header .rescan-action")!;
const crumbs = () =>
  container.querySelector('nav[aria-label="Folder breadcrumbs"]')!;
// The displayed location's full path is the root crumb's tooltip.
const displayedPath = () =>
  crumbs().querySelector<HTMLElement>("button")?.title;
// What the folder picker does with results on screen: select the next Scan target.
async function chooseLocation(target: Target) {
  await act(() => {
    app().setTarget(target);
    app().setHistoricalSource(false);
  });
  await settle();
}
const chooseAlpha = () => chooseLocation(alpha);
const chooseBeta = () => chooseLocation(beta);
beforeEach(() => {
  startupSaved = false;
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command, args) => {
    const current = args?.scanId === "live-beta" ? beta : alpha;
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
        return [alpha, beta];
      case "list_saved_scans":
        return {
          entries: [saved],
          lastOpenedId: startupSaved ? saved.id : null,
        };
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
      case "choose_folder":
        return beta;
      case "start_scan":
        return `live-${args.targetId}`;
      case "scan_summary":
        return summary(args.scanId);
      case "open_saved_scan":
        return summary("historical-alpha", "saved");
      case "rescan_saved_scan":
        return { scanId: "live-alpha", target: alpha };
      case "get_node":
        return node(current, args.nodeId === 1);
      case "get_map":
        return { nodes: [node(current, true)] };
      case "get_children":
        return { items: [node(current, true)], total: 1 };
      default:
        return null;
    }
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});

it("keeps displayed live scan, selection and source when another location is selected", async () => {
  await mount();
  await act(() => app().start());
  await settle();
  await act(() => app().setSelection([node(alpha, true)]));
  const displayed = app().summary;
  const rows = app().page;
  await chooseBeta();
  expect(displayedPath()).toBe(alpha.path);
  expect(
    crumbs().querySelector('[aria-current="page"]')?.textContent?.trim(),
  ).toBe(alpha.name);
  expect(app().target).toEqual(beta);
  expect(button("Scan Projects")).toBeTruthy();
  expect(app().summary).toBe(displayed);
  expect(app().page).toBe(rows);
  expect(app().selection[0].path).toBe(node(alpha, true).path);
  expect(calls("start_scan")).toHaveLength(1);
  await act(() => button("Scan Projects").click());
  await settle();
  expect(calls("start_scan").at(-1)?.[1].targetId).toBe(beta.id);
  expect(displayedPath()).toBe(beta.path);
  expect(container.querySelector(".pending-scan")).toBeNull();
});
it("treats a picked folder as a pending scan without relabeling existing results", async () => {
  await mount();
  await act(() => app().start());
  await settle();
  await act(() => app().chooseFolder());
  await settle();
  expect(app().target).toEqual(beta);
  expect(displayedPath()).toBe(alpha.path);
  expect(button("Scan Projects")).toBeTruthy();
  expect(calls("start_scan")).toHaveLength(1);
});
it("keeps historical identity and restrictions after selecting a different next target", async () => {
  startupSaved = true;
  await mount();
  expect(app().historical).toBe(true);
  await chooseBeta();
  expect(displayedPath()).toBe(alpha.path);
  expect(
    container.querySelector(".scan-status-line .status-chip")?.textContent,
  ).toContain("Saved scan");
  expect(app().target).toEqual(beta);
  expect(
    container.querySelector<HTMLElement>("#selection-inspector")?.hidden,
  ).toBe(false);
  expect(container.textContent).not.toContain("Read-only");
  expect(button("Add to cleanup queue").disabled).toBe(true);
  expect(menuItem("trash").enabled).toBe(false);
  await chooseAlpha();
  expect(calls("rescan_saved_scan")).toHaveLength(0);
  expect(calls("start_scan")).toHaveLength(0);
  await act(() => button("Rescan Projects").click());
  await settle();
  expect(calls("rescan_saved_scan").at(-1)?.[1].scanId).toBe(
    "historical-alpha",
  );
  expect(calls("start_scan")).toHaveLength(0);
});
it("scans the explicitly selected target from historical results", async () => {
  startupSaved = true;
  await mount();
  await chooseBeta();
  await act(() => button("Scan Projects").click());
  await settle();
  expect(calls("rescan_saved_scan")).toHaveLength(0);
  expect(calls("start_scan").at(-1)?.[1].targetId).toBe(beta.id);
  expect(app().historical).toBe(false);
  expect(displayedPath()).toBe(beta.path);
});
it("keeps original-location rescan for an opened saved scan without an explicit new selection", async () => {
  startupSaved = true;
  await mount();
  expect(container.querySelector(".pending-scan")).toBeNull();
  await act(() => button("Rescan Projects").click());
  await settle();
  expect(calls("rescan_saved_scan").at(-1)?.[1].scanId).toBe(
    "historical-alpha",
  );
  expect(calls("start_scan")).toHaveLength(0);
});
it.each([false, true])(
  "preserves the displayed workspace when scan startup rejects (saved=%s)",
  async (historical) => {
    startupSaved = historical;
    await mount();
    if (!historical) await act(() => app().start());
    await settle();
    await act(() => app().setSelection([node(alpha, true)]));
    await settle();
    const previous = {
      summary: app().summary,
      focus: app().focus,
      crumbs: app().crumbs,
      selection: app().selection,
      page: app().page,
      map: app().map,
      source: app().liveTarget,
      historicalSource: app().historicalSource,
    };
    const pending = deferred<string>();
    const standard = mock.invoke.getMockImplementation()!;
    const command = historical ? "rescan_saved_scan" : "start_scan";
    mock.invoke.mockImplementation((name, args) =>
      name === command ? pending.promise : standard(name, args),
    );
    let starting!: Promise<void>;
    await act(() => {
      starting = historical
        ? app().start(null, previous.summary!.scanId)
        : app().start(beta);
    });
    expect(app().summary).toBe(previous.summary);
    expect(app().busy).toBe(true);
    await act(async () => {
      pending.reject(new Error("Target changed. Choose the folder again."));
      await starting;
    });
    expect(app().error).toContain("Target changed");
    for (const key of [
      "summary",
      "focus",
      "crumbs",
      "selection",
      "page",
      "map",
    ] as const)
      expect(app()[key]).toBe(previous[key]);
    expect(app().liveTarget).toBe(previous.source);
    expect(app().historicalSource).toBe(previous.historicalSource);
    expect(app().historical).toBe(historical);
    expect(app().busy).toBe(false);
    expect(displayedPath()).toBe(alpha.path);
  },
);
it("replaces the workspace on the first scan event before the start response", async () => {
  await mount();
  await act(() => app().start());
  await settle();
  await act(() => app().setSelection([node(alpha, true)]));
  const previous = app().summary;
  const pending = deferred<string>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((name, args) =>
    name === "start_scan" ? pending.promise : standard(name, args),
  );
  let starting!: Promise<void>;
  await act(() => {
    starting = app().start(beta);
  });
  expect(app().summary).toBe(previous);
  await act(() =>
    calls("start_scan").at(-1)![1].onEvent.onmessage(summary("live-beta")),
  );
  await settle();
  expect(app().summary?.scanId).toBe("live-beta");
  expect(app().selection).toEqual([]);
  expect(displayedPath()).toBe(beta.path);
  const map = app().map;
  await act(async () => {
    pending.resolve("live-beta");
    await starting;
  });
  expect(app().summary?.scanId).toBe("live-beta");
  expect(app().map).toBe(map);
});

it.each([false, true])(
  "selects a next target without replacing the displayed workspace (saved=%s)",
  async (historical) => {
    startupSaved = historical;
    await mount();
    if (!historical) await act(() => app().start());
    await settle();
    await act(() => app().setSelection([node(alpha, true)]));
    const before = {
      summary: app().summary,
      page: app().page,
      map: app().map,
      focus: app().focus,
      selection: app().selection,
    };
    const starts = calls("start_scan").length;
    await chooseBeta();
    expect(app().target).toEqual(beta);
    expect(rescanAction().textContent?.trim()).toBe("Scan");
    expect(
      container.querySelector('select[aria-label="Next scan target"]'),
    ).toBeNull();
    expect(button("Scan Projects")).toBeTruthy();
    expect(container.querySelector(".pending-scan")).toBeNull();
    for (const key of ["summary", "page", "map", "focus", "selection"] as const)
      expect(app()[key]).toBe(before[key]);
    expect(calls("start_scan")).toHaveLength(starts);
    expect(calls("rescan_saved_scan")).toHaveLength(0);
    await chooseAlpha();
    expect(button("Rescan Projects")).toBeTruthy();
    expect(rescanAction().textContent?.trim()).toBe("Rescan");
    expect(displayedPath()).toBe(alpha.path);
    await act(() => rescanAction().click());
    await settle();
    if (historical) {
      expect(calls("rescan_saved_scan").at(-1)?.[1].scanId).toBe(
        "historical-alpha",
      );
      expect(calls("start_scan")).toHaveLength(starts);
    } else {
      expect(calls("start_scan").at(-1)?.[1].targetId).toBe(alpha.id);
    }
  },
);

it("rescans a saved source that is absent from native targets", async () => {
  startupSaved = true;
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "discover_targets"
      ? Promise.resolve([beta])
      : standard(command, args),
  );
  await mount();
  expect(app().targets.some((item) => item.path === alpha.path)).toBe(false);
  expect(button("Rescan Projects")).toBeTruthy();
  expect(displayedPath()).toBe(alpha.path);
  await act(() => rescanAction().click());
  await settle();
  expect(calls("rescan_saved_scan").at(-1)?.[1].scanId).toBe(
    "historical-alpha",
  );
  expect(calls("start_scan")).toHaveLength(0);
});

it("restores the toolbar and workspace when choosing a folder is cancelled", async () => {
  startupSaved = true;
  await mount();
  const before = {
    target: app().target,
    summary: app().summary,
    historicalSource: app().historicalSource,
    selection: app().selection,
  };
  const choosing = deferred<Target | null>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "choose_folder" ? choosing.promise : standard(command, args),
  );
  const picker = () =>
    container.querySelector<HTMLButtonElement>("button.welcome-target")!;
  await act(() => app().showHome());
  await act(() => picker().click());
  expect(picker().getAttribute("aria-busy")).toBe("true");
  expect(app().can.rescan).toBe(false);
  await act(async () => {
    choosing.resolve(null);
    await choosing.promise;
  });
  await settle();
  expect(picker().getAttribute("aria-busy")).toBe("false");
  expect(app().can.rescan).toBe(true);
  await act(() => app().showResults());
  expect(button("Rescan Projects")).toBeTruthy();
  for (const key of [
    "target",
    "summary",
    "historicalSource",
    "selection",
  ] as const)
    expect(app()[key]).toBe(before[key]);
  expect(calls("start_scan")).toHaveLength(0);
  expect(calls("rescan_saved_scan")).toHaveLength(0);
});
