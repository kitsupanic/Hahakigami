// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { guardianStored } from "./theme.svelte";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import { commandIds, textFieldIds } from "./appMenu";
import { installed, menuItem, menuItems } from "./fakeMenu";
import type { CheckMenuItem } from "./fakeMenu";
import type { NodeView, ScanSummary } from "./types";

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
    onmessage = () => {};
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("./DiskMap.svelte", async () => ({
  default: (await import("./TestMap.svelte")).default,
}));
const app = () => mock.app!;
const root0: NodeView = {
  id: 0,
  parentId: null,
  name: "Home",
  path: "/fixture",
  kind: "directory",
  allocatedBytes: 100,
  childrenCount: 2,
  incomplete: false,
};
const folder: NodeView = {
  ...root0,
  id: 1,
  parentId: 0,
  name: "Photos",
  path: "/fixture/Photos",
  childrenCount: 1,
};
const file: NodeView = {
  ...root0,
  id: 2,
  parentId: 0,
  name: "report.pdf",
  path: "/fixture/report.pdf",
  kind: "file",
  childrenCount: 0,
};
const summary = (): ScanSummary => ({
  scanId: "scan",
  rootId: 0,
  state: "completed",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 3,
  allocatedBytes: 100,
  elapsedMs: 1000,
  errors: 0,
});
let container: HTMLDivElement, root: Root;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
async function flush() {
  await act(async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
  });
}
const button = (label: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (node) =>
      node.getAttribute("aria-label") === label ||
      node.textContent?.trim() === label ||
      (label === "Scan location" && node.classList.contains("welcome-scan")),
  )!;
async function click(item: { click(): void }) {
  await act(async () => item.click());
  await flush();
}
async function select(name: string) {
  const row = [...container.querySelectorAll<HTMLElement>(".rows .row")].find(
    (el) => el.querySelector(".file-name")?.textContent === name,
  )!;
  await act(() =>
    row.dispatchEvent(new MouseEvent("click", { bubbles: true })),
  );
  await flush();
}
const enabled = (id: string) => menuItem(id).enabled;
async function scanned() {
  await act(() => button("Scan location").click());
  await flush();
}
beforeEach(async () => {
  vi.useFakeTimers();
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command, args = {}) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return {
          status: "on",
          scanAnyway: false,
          appPath: "/Fixture.app",
        };
      case "list_cleanup_queue":
        return { entries: [], revision: 0, blocked: false, warning: null };
      case "discover_targets":
        return [
          {
            id: "home",
            name: "Home",
            path: "/fixture",
            totalBytes: null,
            freeBytes: null,
          },
          {
            id: "disk",
            name: "Macintosh HD",
            path: "/",
            totalBytes: 1000,
            freeBytes: 500,
            volume: true,
          },
        ];
      case "start_scan":
        return "scan";
      case "scan_summary":
        return summary();
      case "get_node":
        return args.nodeId === 1 ? folder : root0;
      case "get_children":
        return { items: [folder, file], total: 2 };
      case "get_map":
        return { nodes: [root0, folder, file] };
      case "search_nodes":
        return { items: [file], total: 1 };
      case "add_cleanup_queue":
        return { entries: [], revision: 1, blocked: false, warning: null };
      case "prepare_trash":
        return { token: "review", items: [file] };
      default:
        return undefined;
    }
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() => root.render(App));
  await flush();
});
afterEach(async () => {
  expect(calls("commit_trash")).toHaveLength(0);
  await act(() => root.unmount());
  container.remove();
  mock.app = null;
  vi.useRealTimers();
});

it("installs the menu bar at startup with every command and no scan available", () => {
  expect(installed.menu).not.toBeNull();
  expect(menuItems().size).toBe(commandIds.length);
  // Nothing is scanned yet, so only location commands are live.
  expect(enabled("chooseFolder")).toBe(true);
  expect(enabled("rescan")).toBe(true);
  expect(enabled("scanAllDisks")).toBe(true);
  for (const id of [
    "stopScan",
    "find",
    "sunburst",
    "back",
    "trash",
    "quickLook",
    "reveal",
    "toggleFileList",
  ])
    expect(enabled(id), id).toBe(false);
  expect(enabled("cleanupQueue")).toBe(true);
});

it("shares one availability flag between a button and its menu item", async () => {
  await scanned();
  expect(button("Back").disabled).toBe(true);
  expect(enabled("back")).toBe(false);
  expect(app().can.goBack).toBe(false);
  expect(app().can.enclosing).toBe(false);
  expect(enabled("enclosing")).toBe(false);
  await act(() => app().open(folder));
  await flush();
  expect(button("Back").disabled).toBe(false);
  expect(enabled("back")).toBe(true);
  expect(app().can.goBack).toBe(true);
  expect(app().can.enclosing).toBe(true);
  expect(container.querySelector('[aria-label="Up"]')).toBeNull();
  expect(enabled("enclosing")).toBe(true);
  await click(menuItem("back"));
  expect(app().focus?.id).toBe(0);
  expect(enabled("forward")).toBe(true);
  expect(button("Forward").disabled).toBe(false);
});

it("derives selection commands from the same flags as the inspector buttons", async () => {
  await scanned();
  for (const id of ["trash", "addToQueue", "openFolder"])
    expect(enabled(id), id).toBe(false);
  expect(app().can.trash).toBe(false);
  // Trash has no permanent button; the menu command and queue review reach it.
  expect(
    [...container.querySelectorAll("button")].some(
      (node) => node.textContent?.trim() === "Move to Trash",
    ),
  ).toBe(false);
  await select("report.pdf");
  expect(app().can.trash).toBe(true);
  expect(enabled("trash")).toBe(true);
  expect(enabled("addToQueue")).toBe(true);
  expect(button("Add to cleanup queue").disabled).toBe(false);
  expect(enabled("openFolder")).toBe(false);
  await select("Photos");
  expect(enabled("openFolder")).toBe(true);
  await click(menuItem("openFolder"));
  expect(app().focus?.id).toBe(1);
  expect(enabled("reveal")).toBe(true);
  expect(button("Reveal in Finder").disabled).toBe(false);
});

it("adds the selection to the cleanup queue from its menu command", async () => {
  await scanned();
  await select("report.pdf");
  expect(menuItem("addToQueue").accelerator).toBe("CmdOrCtrl+Shift+A");
  await click(menuItem("addToQueue"));
  expect(calls("add_cleanup_queue")[0][1]).toEqual({
    scanId: "scan",
    nodeIds: [file.id],
  });
});

it("opens the cleanup queue from its menu command and accelerator", async () => {
  await scanned();
  expect(menuItem("cleanupQueue").accelerator).toBe("CmdOrCtrl+Alt+Q");
  await click(menuItem("cleanupQueue"));
  expect(app().queueOpen).toBe(true);
});

it("moves to Trash only through the confirmation dialog", async () => {
  await scanned();
  await select("report.pdf");
  await click(menuItem("trash"));
  expect(calls("prepare_trash")).toHaveLength(1);
  expect(app().trash).not.toBeNull();
  expect(container.querySelector('[role="dialog"]')).not.toBeNull();
  expect(calls("commit_trash")).toHaveLength(0);
});

it("disables every app command while a dialog is open and restores them afterwards", async () => {
  await scanned();
  await select("report.pdf");
  await click(menuItem("trash"));
  expect([...menuItems().values()].every((item) => !item.enabled)).toBe(true);
  await act(() => app().setTrash(null));
  await flush();
  expect(enabled("trash")).toBe(true);
  expect(enabled("rescan")).toBe(true);
  await act(() => app().openAccess());
  await flush();
  expect([...menuItems().values()].every((item) => !item.enabled)).toBe(true);
  await act(() => app().closeAccess());
  await flush();
  expect(enabled("rescan")).toBe(true);
});

it("goes quiet while the filters popover is open", async () => {
  await scanned();
  await act(() => button("Filters").click());
  await flush();
  expect([...menuItems().values()].every((item) => !item.enabled)).toBe(true);
  await act(() => button("Filters").click());
  await flush();
  expect(enabled("rescan")).toBe(true);
});

it("goes quiet while a failure modal is open", async () => {
  await scanned();
  expect(enabled("rescan")).toBe(true);
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "choose_folder") throw new Error("Bad folder");
    return standard(command, args);
  });
  await act(() => void app().chooseFolder());
  await flush();
  expect(container.querySelector('[role="dialog"]')).not.toBeNull();
  expect([...menuItems().values()].every((item) => !item.enabled)).toBe(true);
});

it("disables Go and Item commands while a text field has focus", async () => {
  await scanned();
  await select("report.pdf");
  const search = container.querySelector<HTMLInputElement>(
    'input[aria-label="Search file names"]',
  )!;
  expect(enabled("trash")).toBe(true);
  await act(() => search.focus());
  await flush();
  for (const id of textFieldIds) expect(enabled(id), id).toBe(false);
  expect(enabled("find")).toBe(true);
  expect(enabled("rescan")).toBe(true);
  await click(menuItem("trash"));
  expect(calls("prepare_trash")).toHaveLength(0);
  expect(app().trash).toBeNull();
  await act(() => search.blur());
  await flush();
  expect(enabled("trash")).toBe(true);
  await click(menuItem("cleanupQueue"));
  expect(app().queueOpen).toBe(true);
});

it("focuses the search field from Find", async () => {
  await scanned();
  await click(menuItem("find"));
  expect(document.activeElement).toBe(
    container.querySelector('input[aria-label="Search file names"]'),
  );
});

it("keeps check marks and flipping titles in step with the view", async () => {
  await scanned();
  const checked = (id: string) => (menuItem(id) as CheckMenuItem).checked;
  expect([checked("sunburst"), checked("treemap")]).toEqual([true, false]);
  await click(menuItem("treemap"));
  expect(app().mode).toBe("treemap");
  expect([checked("sunburst"), checked("treemap")]).toEqual([false, true]);
  // The toolbar switch announces the same state the menu checks.
  const pressed = (name: string) =>
    container
      .querySelector(
        `[role="group"][aria-label="Map view"] [aria-label="${name}"]`,
      )
      ?.getAttribute("aria-pressed");
  expect([pressed("Treemap"), pressed("Sunburst")]).toEqual(["true", "false"]);
  await click(menuItem("treemap"));
  expect(checked("treemap")).toBe(true);
  await click(menuItem("viewFileTypes"));
  // Choosing File types also colours the map by type: one control, one state.
  expect(app().mapColorMode).toBe("fileType");
  expect([checked("viewFiles"), checked("viewFileTypes")]).toEqual([
    false,
    true,
  ]);
  expect(
    container.querySelector("#file-types-tab")?.getAttribute("aria-selected"),
  ).toBe("true");
  expect(menuItem("toggleFileList").text).toBe("Hide File List");
  await click(menuItem("toggleFileList"));
  expect(menuItem("toggleFileList").text).toBe("Show File List");
  expect(
    container.querySelector<HTMLElement>("#file-list-column")!.hidden,
  ).toBe(true);
  expect(menuItem("toggleSidebar").text).toBe("Hide Sidebar");
  await click(menuItem("toggleSidebar"));
  expect(menuItem("toggleSidebar").text).toBe("Show Sidebar");
  expect(
    container.querySelector<HTMLElement>("#locations-sidebar")!.hidden,
  ).toBe(true);
});

it("toggles and remembers the Guardian theme from the View menu", async () => {
  await scanned();
  const checked = (id: string) => (menuItem(id) as CheckMenuItem).checked;
  expect(checked("guardianTheme")).toBe(false);
  await click(menuItem("guardianTheme"));
  expect(document.documentElement.dataset.theme).toBe("guardian");
  expect(localStorage.getItem("hahakigami.theme")).toBe("guardian");
  expect(checked("guardianTheme")).toBe(true);
  await click(menuItem("guardianTheme"));
  expect(document.documentElement.dataset.theme).toBeUndefined();
  expect(localStorage.getItem("hahakigami.theme")).toBeNull();

  // The Settings dialog's theme swatches drive the same choice.
  await click(
    [...container.querySelectorAll("button")].find(
      (b) => b.getAttribute("aria-label") === "Settings",
    )!,
  );
  const swatch = (label: string) =>
    container.querySelector<HTMLButtonElement>(
      `[aria-label="${label} theme"]`,
    )!;
  expect(swatch("Mac").getAttribute("aria-pressed")).toBe("true");
  await click(swatch("Guardian"));
  expect(document.documentElement.dataset.theme).toBe("guardian");
  expect(swatch("Guardian").getAttribute("aria-pressed")).toBe("true");
  expect(checked("guardianTheme")).toBe(true);
  // Builds before the single Guardian theme stored a version suffix; it still reads as on.
  localStorage.setItem("hahakigami.theme", "guardian-v5");
  expect(guardianStored()).toBe(true);
  await click(swatch("Mac"));
  expect(document.documentElement.dataset.theme).toBeUndefined();
});

it("runs file commands through the same app functions as their buttons", async () => {
  await scanned();
  expect(enabled("rescan")).toBe(true);
  const before = calls("start_scan").length;
  await click(menuItem("rescan"));
  expect(calls("start_scan")).toHaveLength(before + 1);
  await click(menuItem("cleanupQueue"));
  expect(app().queueOpen).toBe(true);
  expect(enabled("back")).toBe(false);
});

it("exposes command availability as app.can flags", async () => {
  const flags = () => ({ ...app().can });
  expect(flags()).toMatchObject({
    rescan: true,
    chooseFolder: true,
    stopScan: false,
    explore: false,
    find: false,
    trash: false,
    goBack: false,
  });
  await scanned();
  expect(flags()).toMatchObject({
    rescan: true,
    stopScan: false,
    explore: true,
    find: true,
    toggleFileList: true,
    reveal: true,
    quickLook: false,
  });
  await click(menuItem("cleanupQueue"));
  expect(flags()).toMatchObject({
    explore: false,
    find: false,
    trash: false,
    reveal: false,
  });
  expect(enabled("sunburst")).toBe(false);
  expect(enabled("find")).toBe(false);
});

// Commands that stay enabled while Home hides a loaded scan. Each must be fine on the
// welcome page: it works on the hidden scan visibly or not at all, or it is global.
const enabledOnHome = [
  // Location commands: they open pickers or start a scan, not touch the explorer.
  "chooseFolder",
  "rescan", // the welcome page's own Scan button
  "scanAllDisks", // the welcome page's own Scan all disks button
  "cleanupQueue", // leaves Home for the queue
  // Window chrome, unrelated to the results.
  "toggleSidebar",
  "guardianTheme",
];
async function home() {
  await scanned();
  await act(() => app().showHome());
  await flush();
  expect(app().homeOpen).toBe(true);
}

it("enables only allowlisted commands while Home hides a scan", async () => {
  await home();
  const now = commandIds.filter((id) => enabled(id));
  expect(
    now.slice().sort(),
    "New command enabled on Home: decide whether it belongs on the welcome page and add it to enabledOnHome, or gate it on !homeOpen.",
  ).toEqual(enabledOnHome.slice().sort());
});

it.each([
  ["chooseFolder", "choose_folder"],
  ["rescan", "start_scan"],
  ["scanAllDisks", "start_scan"],
])("runs %s from Home instead of doing nothing", async (id, command) => {
  await home();
  const before = calls(command).length;
  await click(menuItem(id));
  expect(calls(command)).toHaveLength(before + 1);
});

it("opens the cleanup queue from Home", async () => {
  await home();
  await click(menuItem("cleanupQueue"));
  expect(app().queueOpen).toBe(true);
  expect(app().homeOpen).toBe(false);
});

it("closes the queue when Rescan from its page takes the view", async () => {
  await scanned();
  await act(() => app().showQueue(true));
  await flush();
  expect(app().can.rescan).toBe(true);
  const before = calls("start_scan").length;
  await click(menuItem("rescan"));
  expect(calls("start_scan")).toHaveLength(before + 1);
  expect(app().queueOpen).toBe(false);
});

it("keeps the queue page open when it is opened after Rescan, before the slow start is accepted", async () => {
  await scanned();
  const previous = mock.invoke.getMockImplementation()!;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "start_scan") await gate;
    return previous(command, args);
  });
  await click(menuItem("rescan"));
  await act(() => app().showQueue(true));
  release();
  await flush();
  expect(app().queueOpen).toBe(true);
});

it("keeps the sidebar toggle off while a failure modal is open", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "start_scan"
      ? Promise.reject(new Error("Scan unavailable"))
      : previous(command, args),
  );
  expect(app().can.toggleSidebar).toBe(true);
  await scanned();
  expect(container.querySelector('[role="dialog"]')).not.toBeNull();
  expect(app().can.toggleSidebar).toBe(false);
  expect(enabled("toggleSidebar")).toBe(false);
});

it("does not freeze the session while a queue entry's target lookup hangs", async () => {
  const previous = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "cleanup_queue_target"
      ? new Promise(() => {})
      : previous(command, args),
  );
  await scanned();
  void app().scanQueue("a");
  await flush();
  expect(app().queueTargeting).toBe(true);
  expect(app().queueBusy).toBe(false);
  expect(app().can.chooseFolder).toBe(true);
  expect(app().can.rescan).toBe(true);
  const before = calls("cleanup_queue_target").length;
  void app().scanQueue("a");
  await flush();
  expect(calls("cleanup_queue_target")).toHaveLength(before);
});
