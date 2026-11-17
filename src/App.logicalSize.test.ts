// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import { menuItem } from "./fakeMenu";
import { setShowLogicalSize } from "./logicalSize.svelte";
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
  logicalBytes: 100,
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
  origin: "live",
  rootId: 0,
  state: "completed",
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 3,
  allocatedBytes: 100,
  logicalBytes: 100,
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
  localStorage.clear();
  setShowLogicalSize(false);
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
      case "list_saved_scans":
        return { entries: [], lastOpenedId: null };
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
  await act(() => root.unmount());
  container.remove();
  mock.app = null;
  vi.useRealTimers();
});

const logicalText = () =>
  /logical/i.test(container.textContent!) ? container.textContent : null;
const metricGroup = () =>
  container.querySelector('[role="group"][aria-label="Size metric"]');
const settingsSwitch = () =>
  container.querySelector<HTMLInputElement>(
    '[role="dialog"] input[aria-label="Show logical size"]',
  )!;
async function setSwitch(on: boolean) {
  await click(button("Settings"));
  if (settingsSwitch().checked !== on) await click(settingsSwitch());
  await click(button("Done"));
}

it("shows On disk only, with no Logical control or label, by default", async () => {
  await scanned();
  expect(app().metric).toBe("allocated");
  expect(metricGroup()).toBeNull();
  expect(button("Logical")).toBeUndefined();
  expect(logicalText()).toBeNull();
  await select("report.pdf");
  expect(logicalText()).toBeNull();
});

it("leaves Logical size out of the Trash confirmation by default", async () => {
  await scanned();
  await select("report.pdf");
  await click(menuItem("trash"));
  const dialog = container.querySelector('[role="dialog"]')!;
  expect(dialog.textContent).toContain("On disk");
  expect(dialog.textContent).not.toMatch(/logical/i);
});

it("ignores the Logical menu command while the switch is off", async () => {
  await scanned();
  expect(menuItem("logical").enabled).toBe(false);
  await click(menuItem("logical"));
  expect(app().metric).toBe("allocated");
});

it("offers Logical again once Show logical size is turned on", async () => {
  await scanned();
  await setSwitch(true);
  expect(metricGroup()).not.toBeNull();
  expect(menuItem("logical").enabled).toBe(true);
  await click(button("Logical"));
  expect(app().metric).toBe("logical");
  expect(container.textContent).toContain("Logical size");
});

it("remembers the choice between launches", async () => {
  await setSwitch(true);
  expect(localStorage.getItem("hahakigami.showLogicalSize")).toBe("true");
  await setSwitch(false);
  expect(localStorage.getItem("hahakigami.showLogicalSize")).toBeNull();
});

it("switches to On disk when turned off while viewing Logical", async () => {
  await scanned();
  await setSwitch(true);
  await click(button("Logical"));
  expect(app().metric).toBe("logical");
  await setSwitch(false);
  expect(app().metric).toBe("allocated");
  expect(metricGroup()).toBeNull();
  expect(logicalText()).toBeNull();
});
