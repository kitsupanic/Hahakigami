// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setShowLogicalSize } from "./logicalSize.svelte";
import App from "./App.svelte";
import { menuItem } from "./fakeMenu";
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
  logicalBytes: 100,
  allocatedBytes: 100,
  childrenCount: 2,
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
const summary = (
  id = "scan-1",
  state: ScanSummary["state"] = "completed",
): ScanSummary => ({
  workers: 8,
  workerStats: [],
  finalizationUs: 0,
  scanId: id,
  rootId: 0,
  state,
  entries: 2,
  logicalBytes: 100,
  allocatedBytes: 100,
  elapsedMs: 1000,
  errors: 0,
});
let container: HTMLDivElement, root: Root;
async function flush() {
  await act(async () => {
    if (vi.isFakeTimers()) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => setTimeout(r, 0));
  });
}
async function click(name: string) {
  if (name === "Move to Trash") {
    // Trash is reached through the Item menu (Cmd+Backspace); the explorer has no Trash button.
    await act(async () => menuItem("trash").click());
    await flush();
    return;
  }
  const scope =
    name === "Compare scans"
      ? container.querySelector(".comparison-panel") || container
      : container;
  const b = Array.from(scope.querySelectorAll("button")).find(
    (b) =>
      b.getAttribute("aria-label") === name ||
      b.textContent?.trim() === name ||
      // The welcome scan button is named by its visible text, which varies ("Scan Home").
      (name === "Scan location" && b.classList.contains("welcome-scan")),
  );
  expect(b, `button ${name}`).toBeTruthy();
  await act(async () => {
    b!.click();
  });
  await flush();
}
function expectHistoricalStatus(historical = true) {
  const line = container.querySelector(".scan-status-line");
  const chip = line?.querySelector(".status-chip.is-saved");
  expect(chip !== null).toBe(historical);
  if (historical) {
    expect(chip?.textContent).toBe("Saved scan");
    expect(line?.querySelector(".is-unsaved")).toBeNull();
    expect(line?.querySelector('[role="status"]')).toBeNull();
    expect(line?.textContent).not.toContain("Read-only");
  }
}
const rescanAction = () =>
  container.querySelector<HTMLButtonElement>("header .rescan-action")!;
const displayedPath = () =>
  container.querySelector<HTMLElement>(
    'nav[aria-label="Folder breadcrumbs"] button',
  )?.title;
const currentCrumb = () =>
  container.querySelector(
    'nav[aria-label="Folder breadcrumbs"] [aria-current="page"]',
  )?.textContent;
// The folder picker is the only way to change the next Scan target with results on screen.
const locationTarget = (text: string) => ({
  click() {
    const chosen = mock.app!.targets.find(
      (item) => item.name.includes(text) || item.path === text,
    )!;
    mock.app!.setTarget(chosen);
    mock.app!.setHistoricalSource(false);
  },
});
async function chooseFolderFromHome() {
  await act(() => mock.app!.showHome());
  await act(() =>
    container
      .querySelector<HTMLButtonElement>("button.welcome-target")!
      .click(),
  );
  await flush();
}
async function savedAction(name: "Export scan" | "Remove scan") {
  const trigger = container.querySelector<HTMLButtonElement>(
    ".saved-menu-trigger",
  )!;
  await act(() => trigger.click());
  await flush();
  await click(name);
}
beforeEach(async () => {
  setShowLogicalSize(true);
  mock.channels.length = 0;
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      switch (command) {
        case "get_explorer_preferences":
          return { sort: "sizeDescending", warning: null };
        case "set_explorer_sort":
          return { sort: args.sort, warning: null };
        case "list_saved_scans":
          return { entries: [], lastOpenedId: null };
        case "full_disk_access_status":
          return {
            status: "on",
            scanAnyway: false,
            appPath: "/Applications/Hahakigami.app",
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
            {
              id: "other",
              name: "Other",
              path: "/other",
              totalBytes: 1000,
              freeBytes: 500,
            },
          ];
        case "start_scan":
          return "scan-1";
        case "scan_summary":
          return summary();
        case "get_node":
          return args.nodeId === 1 ? file : rootNode;
        case "get_children":
        case "search_nodes":
          return { items: [file], total: 1 };
        case "get_map":
          return { nodes: [rootNode, file], rootId: 0 };
        case "prepare_trash":
          return { token: "approved", items: [file] };
        case "commit_trash":
          return { trashed: 1, failures: [] };
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
  restorePlatform();
  restorePlatform = () => {};
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
});
it.each([
  {
    owner: true,
    allocation: 100,
    deduction: 0,
  },
  {
    owner: false,
    allocation: 36,
    deduction: 64,
  },
  {
    owner: false,
    allocation: 0,
    deduction: 100,
  },
])(
  "keeps a zero-attributed clone file visible and trashable ($allocation)",
  async ({ owner, allocation, deduction }) => {
    const original = mock.invoke.getMockImplementation()!;
    const clone: NodeView = {
      ...file,
      allocatedBytes: allocation,
      rawAllocatedBytes: 100,
      verifiedClone: true,
      cloneOwner: owner,
      cloneDeductionBytes: deduction,
    };
    mock.invoke.mockImplementation(
      async (command: string, args: Record<string, unknown>) => {
        if (command === "scan_summary")
          return {
            ...summary(),
            accountingModel: "fullClonesV1",
            accountingWarnings: [
              "Some clone metadata could not be verified; raw allocation was retained.",
            ],
          };
        if (command === "get_node" && args.nodeId === 1) return clone;
        if (command === "get_children" || command === "search_nodes")
          return { items: [clone], total: 1 };
        if (command === "get_map")
          return { nodes: [rootNode, clone], rootId: 0 };
        return original(command, args);
      },
    );
    await click("Scan location");
    await act(async () =>
      (container.querySelector("[role=treeitem]") as HTMLButtonElement).click(),
    );
    await flush();
    await click("Move to Trash");
    expect(container.querySelector("[role=dialog]")?.textContent).toContain(
      clone.path,
    );
    await click("Cancel");
    expect(
      mock.invoke.mock.calls.some(([command]) => command === "commit_trash"),
    ).toBe(false);
  },
);
it("ignores a superseded scan channel and retains current completion", async () => {
  await click("Scan location");
  await click("Rescan Home");
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary("old", "failed"),
      message: "STALE FAILURE",
    }),
  );
  expect(container.textContent).not.toContain("STALE FAILURE");
  expect(container.textContent).toContain("Scan complete");
});
it("requires exact Trash confirmation and cancellation never commits", async () => {
  await click("Scan location");
  const row = container.querySelector("[role=treeitem]") as HTMLButtonElement;
  await act(async () => row.click());
  await click("Move to Trash");
  expect(container.querySelector("[role=dialog]")?.textContent).toContain(
    "/fixture/large.dat",
  );
  await click("Cancel");
  expect(container.querySelector("[role=dialog]")).toBeNull();
  expect(mock.invoke.mock.calls.some(([c]) => c === "commit_trash")).toBe(
    false,
  );
});
it("rescans the original scan target after Trash even when selector changes", async () => {
  await click("Scan location");
  await act(async () => locationTarget("Other").click());
  await act(async () => {
    (container.querySelector("[role=treeitem]") as HTMLButtonElement).click();
  });
  await click("Move to Trash");
  const confirm = container.querySelector(
    "[role=dialog] .danger",
  ) as HTMLButtonElement;
  await act(async () => confirm.click());
  await flush();
  const starts = mock.invoke.mock.calls.filter(([c]) => c === "start_scan");
  expect(starts.at(-1)?.[1].targetId).toBe("home");
});
it("shows partial cancellation and prevents Trash", async () => {
  await click("Scan location");
  await act(async () =>
    mock.channels[0].onmessage(summary("scan-1", "cancelled")),
  );
  expect(container.textContent).toContain("Scan cancelled · partial results");
  await flush();
  expect(menuItem("trash").enabled).toBe(false);
});
it("navigates into folders and retains a root breadcrumb", async () => {
  const dir = {
    ...file,
    id: 7,
    name: "Projects",
    kind: "directory",
    childrenCount: 1,
  } as NodeView;
  const implementation = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "get_children" && args.nodeId === 0)
        return { items: [dir], total: 1 };
      if (command === "get_node" && args.nodeId === 7) return dir;
      return implementation(command, args);
    },
  );
  await click("Scan location");
  await act(async () =>
    container
      .querySelector("[role=treeitem]")!
      .dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
  );
  await flush();
  expect(
    container.querySelector('nav[aria-label="Folder breadcrumbs"]')
      ?.textContent,
  ).toContain("Home");
  expect(
    container.querySelector('nav[aria-label="Folder breadcrumbs"]')
      ?.textContent,
  ).toContain("Projects");
  expect(
    mock.invoke.mock.calls.some(
      ([c, a]) => c === "get_children" && a.nodeId === 7,
    ),
  ).toBe(true);
});
it("searches names through the backend and resets pagination", async () => {
  await click("Scan location");
  const input = container.querySelector("input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, "large");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 270));
  });
  await flush();
  expect(
    mock.invoke.mock.calls.some(
      ([c, a]) => c === "search_nodes" && a.query === "large" && a.offset === 0,
    ),
  ).toBe(true);
});

it("keeps Trash confirmation serialized and rescans after cleanup", async () => {
  await click("Scan location");
  await act(async () =>
    (container.querySelector("[role=treeitem]") as HTMLButtonElement).click(),
  );
  await click("Move to Trash");
  expect(container.querySelector("[role=dialog]")).toBeTruthy();
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "commit_trash"),
  ).toBe(false);
  await act(async () =>
    (container.querySelector(".modal .danger") as HTMLButtonElement).click(),
  );
  await flush();
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "commit_trash"),
  ).toHaveLength(1);
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "start_scan"),
  ).toHaveLength(2);
});
it("acknowledges cancellation immediately while native reads finish, then retains the terminal state", async () => {
  mock.invoke.mockImplementation(async (command: string) => {
    if (command === "discover_targets")
      return [
        {
          id: "home",
          name: "Home",
          path: "/fixture",
          totalBytes: 1000,
          freeBytes: 500,
        },
      ];
    if (command === "start_scan") return "scan-1";
    if (command === "scan_summary") return summary("scan-1", "running");
    if (command === "get_node") return rootNode;
    if (command === "get_children" || command === "search_nodes")
      return { items: [file], total: 1 };
    if (command === "get_map") return { nodes: [rootNode, file], rootId: 0 };
    return undefined;
  });
  await click("Scan location");
  expect(rescanAction().classList.contains("primary")).toBe(true);
  await click("Stop");
  expect(container.textContent).toContain("Stopping scan…");
  expect(
    Array.from(container.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Stopping…",
    )?.disabled,
  ).toBe(true);
  await act(async () =>
    mock.channels.at(-1)!.onmessage(summary("scan-1", "cancelled")),
  );
  await flush();
  expect(container.textContent).toContain("Scan cancelled · partial results");
  expect(container.textContent).not.toContain("Stopping scan…");
});

it("swaps the secondary Rescan button for a filled Stop while a live scan runs", async () => {
  await click("Scan location");
  const action = rescanAction();
  expect(action.textContent?.trim()).toBe("Rescan");
  expect(action.getAttribute("aria-label")).toBe("Rescan Home");
  expect(action.classList.contains("primary")).toBe(false);
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "scan_summary"
      ? summary("scan-1", "running")
      : original(command, args),
  );
  await click("Rescan Home");
  expect(rescanAction().textContent?.trim()).toBe("Stop");
  expect(rescanAction().classList.contains("primary")).toBe(true);
  expect(rescanAction().title).toBe("Stop scan");
  expect(rescanAction().disabled).toBe(false);
  expect(container.querySelector(".scan-status-line")?.textContent).toContain(
    "Scanning…",
  );
  await click("Stop");
  expect(
    mock.invoke.mock.calls.some(
      ([command, args]) =>
        command === "cancel_scan" && args.scanId === "scan-1",
    ),
  ).toBe(true);
  await act(async () =>
    mock.channels.at(-1)!.onmessage(summary("scan-1", "cancelled")),
  );
  await flush();
  expect(rescanAction().textContent?.trim()).toBe("Rescan");
  expect(rescanAction().classList.contains("primary")).toBe(false);
});
it("shows one toolbar row of navigation, breadcrumbs, search, scan action and inspector toggle in focus order", async () => {
  await click("Scan location");
  const header = container.querySelector("header.window-toolbar")!;
  const names = [...header.querySelectorAll("button, input")].map(
    (el) => el.getAttribute("aria-label") ?? el.textContent?.trim(),
  );
  expect(names).toEqual([
    "Back",
    "Forward",
    "Home",
    "Search file names",
    "Rescan Home",
    "Hide file list",
  ]);
  expect(
    container.querySelectorAll('nav[aria-label="Folder breadcrumbs"]'),
  ).toHaveLength(1);
  expect(currentCrumb()?.trim()).toBe("Home");
  expect(header.querySelector("h1, .header-target-path")).toBeNull();
  expect(container.querySelector(".workspace-toolbar")).toBeNull();
  for (const removed of [
    'select[aria-label="Next scan target"]',
    '[aria-label="Up"]',
  ])
    expect(container.querySelector(removed)).toBeNull();
  expect(
    [...container.querySelectorAll("button")].some(
      (b) => b.textContent?.trim() === "Save Scan",
    ),
  ).toBe(false);
  expect(
    container.querySelector("#search-scope-description")?.textContent,
  ).toContain("Search covers all file names in Home");
});
it("offers Save from the status line for a live scan that is not saved", async () => {
  await click("Scan location");
  const line = container.querySelector(".scan-status-line")!;
  expect(line.querySelector(".status-chip.is-unsaved")?.textContent).toBe(
    "Save scan",
  );
  expect(line.textContent).toContain("Scan complete");
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "save_scan"),
  ).toBe(false);
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "save_scan"
      ? Promise.resolve(savedEntry)
      : original(command, args),
  );
  await click("Save scan");
  expect(
    mock.invoke.mock.calls.some(
      ([command, args]) => command === "save_scan" && args.scanId === "scan-1",
    ),
  ).toBe(true);
  // Once saved, the chip stops offering Save so a second click can't duplicate it.
  expect(line.querySelector(".status-chip.is-unsaved")).toBeNull();
  const chip = line.querySelector<HTMLButtonElement>(".status-chip.is-saved")!;
  expect(chip.textContent).toBe("Saved");
  // Same button, so keyboard focus is not dropped; a second click is inert.
  expect(chip.tagName).toBe("BUTTON");
  expect(chip.getAttribute("aria-disabled")).toBe("true");
  expect(line.querySelector("[role=status]")?.textContent).toContain(
    "Scan saved",
  );
  await act(() => chip.click());
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "save_scan"),
  ).toHaveLength(1);
});
it("ignores the second click of a double-click on Rescan", async () => {
  await click("Scan location");
  const starts = () =>
    mock.invoke.mock.calls.filter(([command]) => command === "start_scan")
      .length;
  const before = starts();
  const button = container.querySelector<HTMLButtonElement>(
    "button.rescan-action",
  )!;
  await act(() =>
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 2 })),
  );
  expect(starts()).toBe(before);
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "cancel_scan"),
  ).toHaveLength(0);
});
it("merges storage totals and scan information into one totals popover", async () => {
  await click("Scan location");
  expect(
    [...container.querySelectorAll(".scan-totals .status-value")].map(
      (v) => v.textContent,
    ),
  ).toEqual(["—", "1.0 seconds", "100 B", "2"]);
  expect(
    container.querySelector(".scan-details-popover .scan-facts dd")?.textContent?.trim(),
  ).toBe("100 B");
  expect(container.querySelector(".scan-information > summary")).toBeNull();
  expect(
    [...container.querySelectorAll("summary")].some(
      (node) => node.textContent?.trim() === "Info",
    ),
  ).toBe(false);
});
it.each(["completed", "cancelled", "failed"] as const)(
  "keeps a newer %s event when the initial summary arrives late",
  async (state) => {
    let resolveSummary!: (value: ScanSummary) => void;
    const pending = new Promise<ScanSummary>((resolve) => {
      resolveSummary = resolve;
    });
    const original = mock.invoke.getMockImplementation()!;
    mock.invoke.mockImplementation((command, args) =>
      command === "scan_summary" ? pending : original(command, args),
    );
    await click("Scan location");
    await act(() => mock.channels.at(-1)!.onmessage(summary("scan-1", state)));
    await flush();
    expect(
      container.querySelector(".scan-status-line")?.getAttribute("data-state"),
    ).toBe(state);
    await act(() => resolveSummary(summary("scan-1", "running")));
    await flush();
    expect(
      container.querySelector(".scan-status-line")?.getAttribute("data-state"),
    ).toBe(state);
  },
);

const appPath = "/Applications/Hahakigami.app";
type AccessStatus = "on" | "off" | "unknown";
const access = { status: "on" as AccessStatus, scanAnyway: false };
let restorePlatform = () => {};
function asMac() {
  const spy = vi
    .spyOn(navigator, "platform", "get")
    .mockReturnValue("MacIntel");
  restorePlatform = () => spy.mockRestore();
}
async function remountWithAccess(
  status: AccessStatus,
  options: { restartFailure?: boolean; mac?: boolean } = {},
) {
  if (options.mac ?? true) asMac();
  access.status = status;
  access.scanAnyway = false;
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "full_disk_access_status") return { ...access, appPath };
      if (command === "scan_without_full_disk_access") {
        access.scanAnyway = true;
        return { ...access, appPath };
      }
      if (
        command === "start_scan" &&
        access.status === "off" &&
        !access.scanAnyway
      )
        throw new Error("FULL_DISK_ACCESS_OFF");
      if (command === "restart_app" && options.restartFailure)
        throw new Error("restart failed");
      return original(command, args);
    },
  );
  await act(async () => root.unmount());
  root = createRoot(container);
  await act(async () => root.render(App));
  await flush();
}
const dialogText = () =>
  container.querySelector('[role="dialog"]')?.textContent?.replace(/\s+/g, " ");
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([c]) => c === command);
async function returnFromSettings() {
  await click("Open System Settings…");
  await act(async () => {
    window.dispatchEvent(new Event("focus"));
  });
  await flush();
}
const settingsRow = () =>
  Array.from(container.querySelectorAll("button")).find((b) =>
    b.getAttribute("aria-label")?.startsWith("Settings"),
  )!;
const warningDot = () => settingsRow().querySelector(".settings-dot");
it("shows no dialog at launch and warns on the Settings row while access is off", async () => {
  await remountWithAccess("off");
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(settingsRow().getAttribute("aria-label")).toBe(
    "Settings: Full Disk Access is off",
  );
  expect(settingsRow().textContent).toContain("Settings");
  expect(warningDot()).not.toBeNull();
});
it("warns on the Settings row when access is unknown", async () => {
  await remountWithAccess("unknown");
  expect(settingsRow().getAttribute("aria-label")).toBe(
    "Settings: Full Disk Access status unknown",
  );
  expect(warningDot()).not.toBeNull();
});
it("shows no warning on the Settings row when access is on", async () => {
  await remountWithAccess("on");
  expect(settingsRow().getAttribute("aria-label")).toBe("Settings");
  expect(warningDot()).toBeNull();
});
it("shows no warning and no Full Disk Access section when not on macOS", async () => {
  await remountWithAccess("off", { mac: false });
  expect(warningDot()).toBeNull();
  await click("Settings");
  expect(dialogText()).toContain("Display");
  expect(dialogText()).not.toContain("Full Disk Access");
  expect(container.querySelector("#access-title")).toBeNull();
  await click("Done");
  expect(container.querySelector('[role="dialog"]')).toBeNull();
});
it("opens Settings with Display and Full Disk Access sections on macOS", async () => {
  await remountWithAccess("on");
  await click("Settings");
  const dialog = container.querySelector('[role="dialog"]')!;
  expect(dialog.querySelector("h2")?.textContent).toBe("Settings");
  const sections = [...dialog.querySelectorAll("section")].map((section) =>
    section.querySelector("h3")?.textContent?.trim(),
  );
  expect(sections).toEqual(["Display", "Full Disk Access"]);
});
it("moves the Theme picker from the sidebar footer into the Settings dialog", async () => {
  await remountWithAccess("on");
  const swatch = (label: string) =>
    container.querySelector<HTMLButtonElement>(`[aria-label="${label} theme"]`);
  expect(swatch("Guardian")).toBeNull();
  await click("Settings");
  expect(
    container.querySelector('[role="dialog"]')!.contains(swatch("Guardian")),
  ).toBe(true);
  await click("Guardian theme");
  expect(document.documentElement.dataset.theme).toBe("guardian");
  expect(swatch("Guardian")!.getAttribute("aria-pressed")).toBe("true");
  await click("Mac theme");
  expect(document.documentElement.dataset.theme).toBeUndefined();
  await click("Done");
  expect(swatch("Guardian")).toBeNull();
});
it("remembers the Light, Dark or System appearance chosen in Settings", async () => {
  await remountWithAccess("on");
  await click("Settings");
  const pressed = (label: string) =>
    [
      ...container.querySelectorAll<HTMLButtonElement>(
        '[aria-label="Appearance"] button',
      ),
    ]
      .find((button) => button.textContent?.trim() === label)!
      .getAttribute("aria-pressed");
  expect(pressed("System")).toBe("true");
  await click("Dark");
  expect(pressed("Dark")).toBe("true");
  expect(pressed("System")).toBe("false");
  expect(localStorage.getItem("hahakigami.appearance")).toBe("dark");
  await click("System");
  expect(pressed("System")).toBe("true");
  expect(localStorage.getItem("hahakigami.appearance")).toBeNull();
  // Guardian is light only for now, so it locks the Appearance choice.
  await click("Guardian theme");
  const appearanceButtons = [
    ...container.querySelectorAll<HTMLButtonElement>(
      '[aria-label="Appearance"] button',
    ),
  ];
  expect(appearanceButtons.every((button) => button.disabled)).toBe(true);
  await click("Mac theme");
  expect(appearanceButtons.some((button) => button.disabled)).toBe(false);
});
it.each([
  ["on", "Settings"],
  ["unknown", "Settings: Full Disk Access status unknown"],
] as const)(
  "labels %s status in the Full Disk Access section",
  async (status, label) => {
    await remountWithAccess(status);
    expect(settingsRow().getAttribute("aria-label")).toBe(label);
    await click("Settings");
    expect(container.querySelector("#access-title")?.textContent).toContain(
      status === "on"
        ? "Full Disk Access is on"
        : "Couldn’t check Full Disk Access",
    );
  },
);
it("blocks starting a scan while off and explains Full Disk Access", async () => {
  await remountWithAccess("off");
  await click("Scan location");
  expect(container.querySelector("#access-title")?.textContent).toBe(
    "Full Disk Access is off",
  );
  expect(dialogText()).toContain(
    "Without it, macOS asks about each protected folder during a scan.",
  );
  expect(container.querySelector(".access-path")?.textContent?.trim()).toBe(
    appPath,
  );
  for (const name of ["Copy", "Open System Settings…", "Scan Anyway", "Cancel"])
    expect(
      Array.from(container.querySelectorAll("button")).some(
        (b) => b.textContent?.trim() === name,
      ),
      name,
    ).toBe(true);
  expect(dialogText()).not.toContain("checkbox");
  expect(
    container.querySelector('input[type="checkbox"]:not([role="switch"])'),
  ).toBeNull();
  expect(calls("start_scan")).toHaveLength(0);
  await click("Cancel");
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(calls("start_scan")).toHaveLength(0);
});
const dialogButtons = () =>
  Array.from(container.querySelectorAll(".access-actions button"))
    .map((b) => b.textContent?.trim())
    .filter((t) => t !== "Copy");
it("shows only the Full Disk Access prompt when a scan is blocked", async () => {
  await remountWithAccess("off");
  await click("Scan location");
  const dialog = container.querySelector('[role="dialog"]')!;
  expect(dialog.querySelector("h2")?.textContent).toContain(
    "Full Disk Access is off",
  );
  expect(dialog.querySelector("section")).toBeNull();
  expect(dialogText()).not.toContain("Display");
  expect(dialogText()).not.toContain("Show logical size");
  expect(dialog.querySelector(".theme-swatch")).toBeNull();
  expect(dialogButtons()).toEqual([
    "Scan Anyway",
    "Cancel",
    "Open System Settings…",
  ]);
});
it("keeps the prompt after returning from System Settings with a scan blocked", async () => {
  await remountWithAccess("off");
  await click("Scan location");
  access.status = "on";
  await returnFromSettings();
  expect(dialogText()).toContain("Full Disk Access is on");
  expect(dialogText()).not.toContain("Display");
});
it("opens Settings from the sidebar row with Done, never Scan Anyway or Cancel", async () => {
  await remountWithAccess("off");
  await click("Settings");
  const dialog = container.querySelector('[role="dialog"]')!;
  expect(dialog.querySelector("h2")?.textContent).toBe("Settings");
  expect(dialogText()).toContain("Display");
  expect(dialogText()).toContain("Full Disk Access is off");
  expect(dialogButtons()).toEqual(["Done", "Open System Settings…"]);
  await click("Done");
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(calls("start_scan")).toHaveLength(0);
});
it("lists the Display rows as Theme, Appearance, Show logical size", async () => {
  await remountWithAccess("on");
  await click("Settings");
  const rows = [...container.querySelectorAll(".settings-row > span")].map(
    (row) => row.textContent?.trim(),
  );
  expect(rows).toEqual([
    "Theme",
    "Appearance",
    "Show logical size",
  ]);
});
it("Scan Anyway sets the override then runs the remembered scan", async () => {
  await remountWithAccess("off");
  await click("Scan location");
  await click("Scan Anyway");
  const order = mock.invoke.mock.calls
    .map(([c]) => c)
    .filter((c) => c === "scan_without_full_disk_access" || c === "start_scan");
  expect(order).toEqual(["scan_without_full_disk_access", "start_scan"]);
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(container.textContent).toContain("Scan complete");
});
it("opens the dialog when the backend rejects a scan as FULL_DISK_ACCESS_OFF", async () => {
  await remountWithAccess("on");
  access.status = "off";
  await click("Scan location");
  expect(calls("start_scan")).toHaveLength(1);
  expect(container.querySelector("#access-title")?.textContent).toBe(
    "Full Disk Access is off",
  );
  expect(container.textContent).not.toContain("FULL_DISK_ACCESS_OFF");
  await click("Scan Anyway");
  expect(calls("start_scan")).toHaveLength(2);
});
it("does not block scans when the status is unknown", async () => {
  await remountWithAccess("unknown");
  await click("Scan location");
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(calls("start_scan")).toHaveLength(1);
});
it("shows the status with only Open System Settings and Done when access is on", async () => {
  await remountWithAccess("on");
  await click("Settings");
  expect(dialogText()).toContain("Full Disk Access is on");
  const names = Array.from(container.querySelectorAll(".access-actions button"))
    .map((b) => b.textContent?.trim())
    .filter((t) => t !== "Copy");
  expect(names).toEqual(["Open System Settings…", "Done"]);
  await click("Done");
  expect(container.querySelector('[role="dialog"]')).toBeNull();
});
it("offers Relaunch when access is still off after returning from Settings", async () => {
  await remountWithAccess("off");
  await click("Scan location");
  await returnFromSettings();
  expect(dialogText()).toContain("Full Disk Access is still off");
  expect(
    container
      .querySelector("[role=dialog] button.primary")
      ?.textContent?.trim(),
  ).toBe("Relaunch");
  expect(calls("restart_app")).toHaveLength(0);
  await click("Relaunch");
  expect(calls("restart_app")).toHaveLength(1);
});
it("shows the restart error when relaunching fails", async () => {
  await remountWithAccess("off", { restartFailure: true });
  await click("Scan location");
  await returnFromSettings();
  await click("Relaunch");
  expect(container.querySelector(".access-error")?.textContent).toContain(
    "restart failed",
  );
});
it("updates the status without offering a relaunch when access turned on", async () => {
  await remountWithAccess("off");
  await click("Scan location");
  access.status = "on";
  await returnFromSettings();
  expect(dialogText()).toContain("Full Disk Access is on");
  expect(dialogText()).not.toContain("Relaunch");
  expect(dialogText()).not.toContain("Scan Anyway");
  expect(calls("start_scan")).toHaveLength(0);
  await click("Done");
  expect(settingsRow().getAttribute("aria-label")).toBe("Settings");
});
it("ignores window focus until the user opened System Settings", async () => {
  await remountWithAccess("off");
  const before = calls("full_disk_access_status").length;
  await act(async () => {
    window.dispatchEvent(new Event("focus"));
  });
  await flush();
  expect(calls("full_disk_access_status")).toHaveLength(before);
});
it("keeps folder choosing and cleanup available whatever the status", async () => {
  await remountWithAccess("on");
  await click("Scan location");
  access.status = "off";
  await click("Settings");
  expect(settingsRow().getAttribute("aria-label")).toBe(
    "Settings: Full Disk Access is off",
  );
  await click("Done");
  await act(async () =>
    (container.querySelector('[role="treeitem"]') as HTMLElement).click(),
  );
  await click("Move to Trash");
  expect(calls("prepare_trash")).toHaveLength(1);
  await click("Cancel");
  await chooseFolderFromHome();
  expect(calls("choose_folder")).toHaveLength(1);
});
it("shows the explorer directly with worker diagnostics and no settings IPC", async () => {
  expect(container.querySelector(".welcome-target strong")!.textContent).toBe(
    "Home",
  );
  await click("Scan location");
  expect(
    container.querySelector<HTMLElement>(".scan-details-popover")!.hidden,
  ).toBe(true);
  expect(
    container.querySelector(".storage-duration dd")!.textContent?.trim(),
  ).toBe("1.0 seconds");
  expect(
    container
      .querySelector(".scan-status-line .scan-workers")!
      .textContent?.replace(/\s+/g, " ")
      .trim(),
  ).toBe("8");
  expect(container.querySelector(".worker-diagnostics")).not.toBeNull();
  expect(
    mock.invoke.mock.calls.some(
      ([command]) =>
        command === "get_scan_settings" || command === "set_scan_workers",
    ),
  ).toBe(false);
});

it("hides expected exclusions without calling them unreadable or incomplete", async () => {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) =>
      command === "scan_summary"
        ? { ...summary(), excluded: 278, incomplete: false }
        : original(command, args),
  );
  await click("Scan location");
  expect(container.querySelector(".scan-status-line")!.textContent).toContain(
    "Scan complete",
  );
  expect(container.textContent).not.toContain("excluded locations");
  expect(container.querySelector('[aria-label="Show scan details"]')).toBeNull();
  expect(container.textContent).not.toContain("skipped or unreadable entries");
  expect(container.querySelector(".scan-issue-warning")).toBeNull();
  expect(container.textContent).not.toContain("Full Disk Access settings ↗");
});
it("labels actual events as scan issues and keeps cancelled results partial", async () => {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) =>
      command === "scan_summary"
        ? { ...summary(), errors: 2, excluded: 1, incomplete: true }
        : original(command, args),
  );
  await click("Scan location");
  expect(container.querySelector(".scan-status-line")!.textContent).toContain(
    "Scan complete · scan issues",
  );
  expect(container.querySelector(".scan-status-row")!.textContent).toContain(
    "2 issue events",
  );
  expect(container.querySelector(".scan-status-row")?.textContent).toContain(
    "Limited coverage",
  );
  expect(container.textContent).not.toContain(
    "For permission errors, enable Hahakigami",
  );
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary("scan-1", "cancelled"),
      errors: 2,
    }),
  );
  expect(container.querySelector(".scan-status-line")!.textContent).toContain(
    "Scan cancelled · partial results",
  );
});

it("describes an excluded directory as outside scope instead of empty and blocks Trash", async () => {
  const original = mock.invoke.getMockImplementation()!;
  const excluded = {
    ...file,
    name: "Other volume",
    kind: "directory",
    path: "/Volumes/Other",
    excluded: true,
    logicalBytes: 0,
    allocatedBytes: 0,
    childrenCount: 0,
  };
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) =>
      command === "get_children"
        ? { items: [excluded], total: 1 }
        : command === "get_node" && args.nodeId === excluded.id
          ? excluded
          : original(command, args),
  );
  await click("Scan location");
  await act(async () =>
    (container.querySelector('[role="treeitem"]') as HTMLElement).click(),
  );
  const inspector = container.querySelector(".inspector")!;
  expect(inspector.querySelector(".details-size")?.textContent).toContain(
    "Excluded",
  );
  expect(inspector.textContent).not.toContain("0 B");
  expect(inspector.textContent).not.toContain("0 items");
  await flush();
  expect(menuItem("trash").enabled).toBe(false);
});

it("shows completed system-only restrictions as a plain complete scan with no coverage chip", async () => {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) =>
      command === "scan_summary"
        ? {
            ...summary(),
            errors: 278,
            restrictedSystemIssues: 278,
            incomplete: true,
          }
        : original(command, args),
  );
  await click("Scan location");
  const line = container.querySelector(".scan-status-line")!.textContent!;
  expect(line).toContain("Scan complete");
  expect(line).not.toContain("restricted");
  expect(container.querySelector(".scan-issue-warning")).toBeNull();
  expect(container.querySelector(".scan-coverage-chip")).toBeNull();
  expect(container.textContent).not.toContain("restricted system locations");
});

const savedEntry = {
  id: "saved-1",
  name: "Home history",
  rootPath: "/fixture",
  startedAtMs: 1000,
  completedAtMs: 2000,
  savedAtMs: 3000,
  entries: 2,
  logicalBytes: 100,
  allocatedBytes: 100,
  incomplete: false,
  errors: 0,
  excluded: 0,
  fileBytes: 80,
};
async function mountHistory(
  restore = true,
  accessOff = false,
  accounting: Partial<ScanSummary> = {},
  entry = savedEntry,
) {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "list_saved_scans")
      return {
        entries: [entry],
        lastOpenedId: restore ? entry.id : null,
      };
    if (command === "open_saved_scan")
      return {
        ...summary("historical"),
        origin: "saved",
        scanMode: "full",
        savedId: entry.id,
        ...accounting,
      };
    if (command === "full_disk_access_status" && accessOff)
      return { status: "off", scanAnyway: false, appPath };
    if (command === "rescan_saved_scan")
      return {
        scanId: "scan-1",
        target: {
          id: "saved-location",
          name: "Saved location",
          path: "/historical-root",
          totalBytes: null,
          freeBytes: null,
        },
      };
    return original(command, args);
  });
  await act(async () => {
    root.unmount();
    root = createRoot(container);
    root.render(App);
  });
  await flush();
}
it("shows a recorded epoch completion time in the historical header", async () => {
  await mountHistory(true, false, {}, { ...savedEntry, completedAtMs: 0 });
  const timestamp = container.querySelector(".scan-timestamp");
  expect(timestamp).not.toBeNull();
  expect(timestamp?.textContent).toContain("Scanned");
  expect(timestamp?.getAttribute("title")).toContain("Scan completed");
});

it("shows saved clone attribution before setup while retaining historical action restrictions", async () => {
  const original = mock.invoke.getMockImplementation()!;
  const clone: NodeView = {
    ...file,
    allocatedBytes: 0,
    rawAllocatedBytes: 100,
    verifiedClone: true,
    cloneOwner: false,
    cloneDeductionBytes: 100,
  };
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "get_children") return { items: [clone], total: 1 };
    if (command === "get_node" && args.nodeId === 1) return clone;
    return original(command, args);
  });
  await mountHistory(true, true, {
    accountingModel: "fullClonesV1",
    accountingWarnings: ["Historical accounting warning"],
  });
  await act(async () =>
    (container.querySelector("[role=treeitem]") as HTMLButtonElement).click(),
  );
  await flush();
  expect(menuItem("trash").enabled).toBe(false);
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "prepare_trash"),
  ).toBe(false);
});
it("restores saved results without a dialog while access is off and blocks only the rescan", async () => {
  asMac();
  await mountHistory(true, true);
  expect(
    container.querySelector<HTMLElement>("#selection-inspector")?.hidden,
  ).toBe(false);
  expectHistoricalStatus();
  expect(
    container.querySelector(".scan-status-line")?.textContent,
  ).not.toContain("full scan");
  expect(container.querySelector("[role=dialog]")).toBeNull();
  const buttons = [...container.querySelectorAll("button")];
  expect(
    buttons.some((b) => b.textContent?.trim() === "Reveal in Finder ↗"),
  ).toBe(false);
  expect(buttons.some((b) => b.textContent?.trim() === "Quick Look")).toBe(
    false,
  );
  expect(buttons.some((b) => b.textContent?.trim() === "Move to Trash")).toBe(
    false,
  );
  expect(menuItem("trash").enabled).toBe(false);
  expect(container.querySelector(".status-chip.is-unsaved")).toBeNull();
  const rescan = rescanAction();
  expect(rescan.disabled).toBe(false);
  await act(() => rescan.click());
  await flush();
  expect(container.querySelector("[role=dialog]")?.textContent).toContain(
    "Full Disk Access is off",
  );
  expect(mock.invoke.mock.calls.some(([c]) => c === "rescan_saved_scan")).toBe(
    false,
  );
});
it("removes the archive and closes the displayed saved scan", async () => {
  await mountHistory();
  await savedAction("Remove scan");
  expect(container.querySelector("[role=dialog]")?.textContent).toContain(
    "Only the stored result",
  );
  await click("Remove saved result");
  expect(mock.invoke.mock.calls).toContainEqual([
    "delete_saved_scan",
    { id: "saved-1" },
  ]);
  // No Current scan sits behind it, so Home is shown with no results.
  expect(container.querySelector(".scan-status-line")).toBeNull();
  expect(container.querySelector(".status-chip.is-saved")).toBeNull();
  expect(container.textContent).toContain("No saved scans yet.");
});
it("imports into the library without replacing the current live results", async () => {
  await click("Scan location");
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "import_saved_scan") return savedEntry;
    return original(command, args);
  });
  await click("Import Scan");
  expect(mock.invoke).toHaveBeenCalledWith("import_saved_scan", {
    onProgress: expect.anything(),
  });
  expect(container.textContent).toContain("Home history");
  expect(container.textContent).toContain("Imported scan added to Saved Scans");
  expect(container.textContent).toContain("Scan complete");
  expectHistoricalStatus(false);
  expect(mock.invoke.mock.calls.some(([c]) => c === "open_saved_scan")).toBe(
    false,
  );
});
it("exports through a native picker using only a saved ID and preserves historical restrictions", async () => {
  await mountHistory(true, true);
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "export_saved_scan") return true;
    return original(command, args);
  });
  await savedAction("Export scan");
  expect(mock.invoke).toHaveBeenCalledWith("export_saved_scan", {
    id: savedEntry.id,
    onProgress: expect.anything(),
  });
  expect(container.textContent).toContain("Scan exported");
  expect(
    container.querySelector(".import-button")?.getAttribute("title"),
  ).toContain("full paths and sizes, but no file contents");
  expectHistoricalStatus();
  expect(menuItem("trash").enabled).toBe(false);
});
it("treats portable dialog cancellation as a no-op without losing results", async () => {
  await mountHistory();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "import_saved_scan") return null;
    if (command === "export_saved_scan") return false;
    return original(command, args);
  });
  await click("Import Scan");
  await savedAction("Export scan");
  expect(container.querySelector('.error[role="alert"]')).toBeNull();
  expect(container.textContent).not.toContain("Scan exported");
  expect(container.querySelectorAll(".saved-entry")).toHaveLength(1);
});
const failureDialog = () =>
  container.querySelector<HTMLElement>('[role="dialog"]');
// jsdom has no `inert` reflection, so read the property Svelte sets.
const inert = (selector: string) =>
  (container.querySelector(selector) as HTMLElement & { inert: boolean }).inert;
const failureButton = (label: string) =>
  [...(failureDialog()?.querySelectorAll("button") ?? [])].find(
    (b) => b.textContent?.trim() === label,
  ) as HTMLButtonElement | undefined;
it("opens a modal for a failed import and recovers by choosing another file", async () => {
  await mountHistory();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "import_saved_scan")
      throw new Error("Invalid shared scan checksum");
    return original(command, args);
  });
  await click("Import Scan");
  const dialog = failureDialog()!;
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  expect(dialog.querySelector("h2")?.textContent).toContain("Import failed");
  // The raw error is only a technical detail, never the main message.
  const details = dialog.querySelector("details")!;
  expect(details.textContent).toContain("Invalid shared scan checksum");
  expect(dialog.textContent?.replace(details.textContent!, "")).not.toContain(
    "Invalid shared scan checksum",
  );
  expect(container.querySelector("main .scan-transfer")).toBeNull();
  expect(inert("main")).toBe(true);
  expect(inert("aside")).toBe(true);
  expectHistoricalStatus();
  expect(container.querySelectorAll(".saved-entry")).toHaveLength(1);
  const before = mock.invoke.mock.calls.filter(
    ([c]) => c === "import_saved_scan",
  ).length;
  await act(() => failureButton("Choose another file")!.click());
  await flush();
  expect(
    mock.invoke.mock.calls.filter(([c]) => c === "import_saved_scan"),
  ).toHaveLength(before + 1);
});
it("opens a modal for a failed export with its reason and a Done button", async () => {
  await mountHistory();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "export_saved_scan") throw new Error("Disk is full");
    return original(command, args);
  });
  await savedAction("Export scan");
  const dialog = failureDialog()!;
  expect(dialog.querySelector("h2")?.textContent).toContain("Export failed");
  expect(dialog.querySelector("details")?.textContent).toContain(
    "Disk is full",
  );
  expect(container.querySelector("main .scan-transfer")).toBeNull();
  await act(() => failureButton("Done")!.click());
  expect(failureDialog()).toBeNull();
  expect(inert("main")).toBe(false);
});
it("keeps the pending transfer indicator out of the explorer layout", async () => {
  await mountHistory();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "import_saved_scan"
      ? new Promise(() => {})
      : original(command, args),
  );
  await click("Import Scan");
  expect(
    container.querySelector(".scan-transfer .transfer-phase"),
  ).not.toBeNull();
  expect(container.querySelector("main .scan-transfer")).toBeNull();
});
const toast = () => container.querySelector<HTMLElement>(".toast");
const toastButton = (label: string) =>
  [...(toast()?.querySelectorAll("button") ?? [])].find(
    (b) =>
      b.textContent?.trim() === label || b.getAttribute("aria-label") === label,
  ) as HTMLButtonElement;
function answerWith(command: string, value: unknown) {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (name, args) =>
    name === command ? value : original(name, args),
  );
}
it.each([
  {
    label: "Import Scan",
    command: "import_saved_scan",
    value: null,
    text: "Import cancelled",
  },
  {
    label: "Export scan",
    command: "export_saved_scan",
    value: false,
    text: "Export cancelled",
  },
  {
    label: "Export scan",
    command: "export_saved_scan",
    value: true,
    text: "Scan exported",
  },
  {
    label: "Import Scan",
    command: "import_saved_scan",
    value: savedEntry,
    text: "Imported scan added to Saved Scans",
  },
])(
  "announces $text in a polite toast without moving focus or adding a card above the map",
  async ({ label, command, value, text }) => {
    await mountHistory(true, true);
    answerWith(command, value);
    const focused = document.activeElement;
    if (label === "Import Scan") await click(label);
    else await savedAction("Export scan");
    expect(toast()?.textContent).toContain(text);
    const region = toast()!.closest("[aria-live]")!;
    expect(region.getAttribute("aria-live")).toBe("polite");
    expect(region.getAttribute("role")).toBe("status");
    expect(container.contains(document.activeElement)).toBe(
      container.contains(focused),
    );
    expect(toast()!.contains(document.activeElement)).toBe(false);
    expect(container.querySelector(".scan-transfer")).toBeNull();
  },
);
it("confirms a saved scan with a toast", async () => {
  await click("Scan location");
  answerWith("save_scan", savedEntry);
  await click("Save scan");
  expect(toast()?.textContent).toContain("Scan saved");
});
it("opens a modal for a failed save, with the raw error only under Technical details", async () => {
  await click("Scan location");
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "save_scan") throw new Error("No space left on device");
    return original(command, args);
  });
  await click("Save scan");
  const dialog = failureDialog()!;
  expect(dialog.querySelector("h2")?.textContent).toContain(
    "Couldn’t save the scan",
  );
  const details = dialog.querySelector("details")!;
  expect(details.textContent).toContain("No space left on device");
  expect(dialog.textContent?.replace(details.textContent!, "")).not.toContain(
    "No space left on device",
  );
  expect(container.querySelector("main .error")).toBeNull();
  expect(toast()).toBeNull();
  expect(inert("main")).toBe(true);
  await act(() => failureButton("Close")!.click());
  expect(failureDialog()).toBeNull();
  expect(inert("main")).toBe(false);
});
it("shows a cancelled scan as a toast instead of a note above the map", async () => {
  await click("Scan location");
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary("scan-1", "cancelled"),
      message: "Scan stopped by the user",
    }),
  );
  expect(toast()?.textContent).toContain("Scan cancelled");
  expect(container.querySelector(".scan-note")).toBeNull();
});
it("opens a modal for a failed scan and offers Scan again", async () => {
  await click("Scan location");
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary("scan-1", "failed"),
      message: "Disk vanished",
    }),
  );
  const dialog = failureDialog()!;
  expect(dialog.querySelector("h2")?.textContent).toContain("Scan failed");
  const details = dialog.querySelector("details")!;
  expect(details.textContent).toContain("Disk vanished");
  expect(dialog.textContent?.replace(details.textContent!, "")).not.toContain(
    "Disk vanished",
  );
  expect(container.querySelector(".scan-note")).toBeNull();
  expect(toast()).toBeNull();
  const starts = () =>
    mock.invoke.mock.calls.filter(([c]) => c === "start_scan").length;
  const before = starts();
  await act(() => failureButton("Scan again")!.click());
  await flush();
  expect(starts()).toBe(before + 1);
  expect(failureDialog()).toBeNull();
});
it("closes the failed scan modal with Escape and does not reopen it for the same scan", async () => {
  await click("Scan location");
  const failed = { ...summary("scan-1", "failed"), message: "Disk vanished" };
  await act(async () => mock.channels[0].onmessage(failed));
  expect(failureDialog()).not.toBeNull();
  await act(async () =>
    failureDialog()!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(failureDialog()).toBeNull();
  await act(async () => mock.channels[0].onmessage({ ...failed, entries: 3 }));
  expect(failureDialog()).toBeNull();
});
it("fades a toast after about four seconds, unless hovered or focused", async () => {
  await mountHistory(true, true);
  answerWith("export_saved_scan", true);
  vi.useFakeTimers();
  try {
    await savedAction("Export scan");
    await act(() => vi.advanceTimersByTimeAsync(3900));
    expect(toast()).not.toBeNull();
    await act(() => vi.advanceTimersByTimeAsync(300));
    expect(toast()).toBeNull();
    await act(() => toastlessExport());
    await act(() =>
      toast()!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })),
    );
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(toast()).not.toBeNull();
    await act(() =>
      toast()!.dispatchEvent(new MouseEvent("mouseout", { bubbles: true })),
    );
    await act(() => vi.advanceTimersByTimeAsync(4100));
    expect(toast()).toBeNull();
    await act(() => toastlessExport());
    await act(() =>
      toast()!.dispatchEvent(new FocusEvent("focusin", { bubbles: true })),
    );
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(toast()).not.toBeNull();
    await act(() =>
      toast()!.dispatchEvent(new FocusEvent("focusout", { bubbles: true })),
    );
    await act(() => vi.advanceTimersByTimeAsync(4100));
    expect(toast()).toBeNull();
  } finally {
    vi.useRealTimers();
  }
  async function toastlessExport() {
    await savedAction("Export scan");
  }
});
it("dismisses a toast early and lets a new toast replace the current one", async () => {
  await mountHistory(true, true);
  answerWith("export_saved_scan", true);
  await savedAction("Export scan");
  await act(() => toastButton("Dismiss").click());
  expect(toast()).toBeNull();
  await savedAction("Export scan");
  expect(toast()?.textContent).toContain("Scan exported");
  answerWith("export_saved_scan", false);
  await savedAction("Export scan");
  expect(container.querySelectorAll(".toast")).toHaveLength(1);
  expect(toast()?.textContent).toContain("Export cancelled");
  expect(toast()?.textContent).not.toContain("Scan exported");
});
it("keeps the old cards' actions on their toasts", async () => {
  await mountHistory(true, true);
  answerWith("import_saved_scan", savedEntry);
  await click("Import Scan");
  const opens = () =>
    mock.invoke.mock.calls.filter(([c]) => c === "open_saved_scan").length;
  const before = opens();
  await act(() => toastButton("Open imported scan").click());
  await flush();
  expect(opens()).toBe(before + 1);
  expect(toast()).toBeNull();
  answerWith("import_saved_scan", null);
  await click("Import Scan");
  answerWith("import_saved_scan", savedEntry);
  await act(() => toastButton("Choose file again").click());
  await flush();
  expect(toast()?.textContent).toContain("Imported scan added");
});
it("blocks conflicting operations while importing and releases controls after cancellation", async () => {
  await mountHistory();
  let finish!: (value: unknown) => void;
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "import_saved_scan")
      return new Promise((resolve) => {
        finish = resolve;
      });
    return original(command, args);
  });
  await click("Import Scan");
  expect(
    container.querySelector(".scan-transfer .transfer-phase")?.textContent,
  ).toContain("Choose a shared scan file in the file dialog.");
  expect(
    container.querySelector<HTMLButtonElement>(".saved-menu-trigger")?.disabled,
  ).toBe(true);
  for (const name of ["Import Scan", "Rescan Home"]) {
    expect(
      [...container.querySelectorAll("button")].find(
        (b) =>
          b.getAttribute("aria-label") === name ||
          b.textContent?.trim() === name,
      )?.disabled,
    ).toBe(true);
  }
  await act(() =>
    container.querySelector<HTMLButtonElement>(".saved-menu-trigger")!.click(),
  );
  expect(container.querySelector(".saved-menu")).toBeNull();
  expect(mock.invoke.mock.calls.some(([c]) => c === "export_saved_scan")).toBe(
    false,
  );
  await act(async () => finish(null));
  await flush();
  expect(
    [...container.querySelectorAll("button")].find(
      (b) => b.getAttribute("aria-label") === "Import Scan",
    )?.disabled,
  ).toBe(false);
});
it("rescans saved locations through the native trusted-location command", async () => {
  await mountHistory();
  const inspector = container.querySelector<HTMLElement>(
    "#selection-inspector",
  )!;
  expect(inspector.getAttribute("aria-label")).toBe("Current folder details");
  const visibleActions = [
    ...inspector.querySelectorAll<HTMLButtonElement>(
      ".inspector-actions > button",
    ),
  ].filter((button) => !button.hidden);
  expect(visibleActions).toHaveLength(0);
  expect(inspector.querySelector(".historical-context")?.textContent).toContain(
    "can't preview or queue files from a saved scan",
  );
  expect(container.querySelectorAll("header .rescan-action")).toHaveLength(1);
  await click("Rescan Home");
  await flush();
  expect(
    mock.invoke.mock.calls.some(
      ([c, a]) => c === "rescan_saved_scan" && a.scanId === "historical",
    ),
  ).toBe(true);
  expectHistoricalStatus(false);
});
it("shows open errors without replacing the current live result", async () => {
  await mountHistory(false);
  await click("Scan location");
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "open_saved_scan")
      throw new Error("Archive checksum failed");
    return original(command, args);
  });
  const history = [...container.querySelectorAll("button")].find(
    (b) =>
      b.getAttribute("aria-label")?.includes("Home history") &&
      b.classList.contains("saved-summary"),
  )!;
  await act(async () => history.click());
  await flush();
  expect(container.textContent).toContain("Archive checksum failed");
  expect(container.textContent).toContain("Scan complete");
  expectHistoricalStatus(false);
});
it("blocks switching during saving while keeping browsing controls available", async () => {
  await click("Scan location");
  let finish!: (value: unknown) => void;
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "save_scan")
      return new Promise((resolve) => {
        finish = resolve;
      });
    return original(command, args);
  });
  await click("Save scan");
  expect(container.textContent).toContain("Saving scan");
  expect(
    [...container.querySelectorAll("button")].find(
      (b) => b.getAttribute("aria-label") === "Rescan Home",
    )!.disabled,
  ).toBe(true);
  expect(
    [...container.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === "Logical",
    )!.disabled,
  ).toBe(false);
  await act(async () => finish(savedEntry));
  await flush();
  expect(container.textContent).toContain("Home history");
  expect(container.textContent).not.toContain("Saving scan");
});

it("uses the authoritative historical rescan target for cleanup rescanning", async () => {
  await mountHistory();
  await click("Rescan Home");
  expect(displayedPath()).toBe("/historical-root");
  await act(async () =>
    (container.querySelector("[role=treeitem]") as HTMLButtonElement).click(),
  );
  await click("Move to Trash");
  await act(async () =>
    (
      container.querySelector("[role=dialog] .danger") as HTMLButtonElement
    ).click(),
  );
  await flush();
  const starts = mock.invoke.mock.calls.filter(([c]) => c === "start_scan");
  expect(starts.at(-1)?.[1].targetId).toBe("saved-location");
});

it("restores once after remount and retains the opening lock", async () => {
  const original = mock.invoke.getMockImplementation()!;
  const lists: ((value: unknown) => void)[] = [];
  let finish!: (value: unknown) => void;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "list_saved_scans")
      return new Promise((resolve) => lists.push(resolve));
    if (command === "open_saved_scan")
      return new Promise((resolve) => {
        finish = resolve;
      });
    return original(command, args);
  });
  await act(async () => {
    root.unmount();
    root = createRoot(container);
    root.render(App);
    root.unmount();
    root = createRoot(container);
    root.render(App);
  });
  expect(lists).toHaveLength(2);
  await act(async () =>
    lists[1]({ entries: [savedEntry], lastOpenedId: savedEntry.id }),
  );
  await flush();
  await act(async () =>
    lists[0]({ entries: [savedEntry], lastOpenedId: savedEntry.id }),
  );
  await flush();
  expect(
    mock.invoke.mock.calls.filter(([c]) => c === "open_saved_scan"),
  ).toHaveLength(1);
  expect(container.textContent).toContain("Opening saved scan");
  expect(rescanAction().disabled).toBe(true);
  await act(async () =>
    finish({
      ...summary("historical"),
      origin: "saved",
      savedId: savedEntry.id,
    }),
  );
  await flush();
  expectHistoricalStatus();
  expect(container.textContent).not.toContain("Opening saved scan");
});

it("scans a newly selected live location while retaining historical action guards", async () => {
  await mountHistory();
  await act(() => locationTarget("Other").click());
  await flush();
  expect(displayedPath()).toBe("/fixture");
  expect(container.querySelector(".pending-scan")).toBeNull();
  expect(rescanAction().getAttribute("aria-label")).toBe("Scan Other");
  expect(rescanAction().textContent?.trim()).toBe("Scan");
  expect(mock.invoke.mock.calls.some(([c]) => c === "start_scan")).toBe(false);
  expectHistoricalStatus();
  expect(
    [...container.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === "Reveal in Finder ↗",
    ),
  ).toBeUndefined();
  await click("Scan Other");
  expect(
    mock.invoke.mock.calls.some(
      ([c, a]) => c === "start_scan" && a.targetId === "other",
    ),
  ).toBe(true);
  expect(mock.invoke.mock.calls.some(([c]) => c === "rescan_saved_scan")).toBe(
    false,
  );
});
it("scans a picked folder rather than the displayed historical root", async () => {
  await mountHistory();
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "choose_folder"
      ? Promise.resolve({
          id: "picked",
          name: "Picked",
          path: "/picked",
          totalBytes: null,
          freeBytes: null,
        })
      : original(command, args),
  );
  await chooseFolderFromHome();
  await act(() => mock.app!.showResults());
  expect(rescanAction().getAttribute("aria-label")).toBe("Scan Picked");
  expect(displayedPath()).toBe("/fixture");
  expect(
    mock.invoke.mock.calls.some(
      ([c]) => c === "start_scan" || c === "rescan_saved_scan",
    ),
  ).toBe(false);
  await click("Scan Picked");
  expect(
    mock.invoke.mock.calls.some(
      ([c, a]) => c === "start_scan" && a.targetId === "picked",
    ),
  ).toBe(true);
});
it("retains a historical-root rescan action after selecting another location", async () => {
  await mountHistory();
  await act(async () => locationTarget("Other").click());
  await flush();
  expect(container.querySelector(".pending-scan")).toBeNull();
  expect(container.querySelector(".rescan-source-action")).toBeNull();
  expect(rescanAction().getAttribute("aria-label")).toBe("Scan Other");
  await act(() => locationTarget("/fixture").click());
  await flush();
  expect(rescanAction().getAttribute("aria-label")).toBe("Rescan Home");
  expect(
    mock.invoke.mock.calls.some(
      ([c]) => c === "start_scan" || c === "rescan_saved_scan",
    ),
  ).toBe(false);
  await click("Rescan Home");
  expect(
    mock.invoke.mock.calls.some(
      ([c, a]) => c === "rescan_saved_scan" && a.scanId === "historical",
    ),
  ).toBe(true);
});

it("choosing the displayed location again preserves displayed results and rescans the original saved location", async () => {
  await mountHistory();
  await act(() => locationTarget("Other").click());
  await flush();
  const heading = currentCrumb();
  const rows = container.querySelector(".file-list")!.textContent;
  const mapCalls = mock.invoke.mock.calls.filter(
    ([c]) => c === "get_map",
  ).length;
  await act(() => locationTarget("/fixture").click());
  await flush();
  expect(currentCrumb()).toBe(heading);
  expect(container.querySelector(".file-list")!.textContent).toBe(rows);
  expect(mock.invoke.mock.calls.filter(([c]) => c === "get_map")).toHaveLength(
    mapCalls,
  );
  expect(
    mock.invoke.mock.calls.some(
      ([c]) => c === "start_scan" || c === "rescan_saved_scan",
    ),
  ).toBe(false);
  const rescan = rescanAction();
  expect(rescan.getAttribute("aria-label")).toBe("Rescan Home");
  expect(rescan.textContent?.trim()).toBe("Rescan");
  await act(() => rescan.click());
  await flush();
  expect(
    mock.invoke.mock.calls.some(
      ([command, args]) =>
        command === "rescan_saved_scan" && args.scanId === "historical",
    ),
  ).toBe(true);
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "start_scan"),
  ).toBe(false);
});

it("the inspector returns to current-folder context when an explicit selection clears", async () => {
  await mountHistory();
  const inspector = container.querySelector<HTMLElement>(
    "#selection-inspector",
  )!;
  const row = container.querySelector<HTMLElement>(
    '.file-list [role="treeitem"]',
  )!;
  await act(() => row.click());
  expect(inspector.hidden).toBe(false);
  expect(inspector.getAttribute("aria-label")).toBe("Selection details");
  expect(inspector.querySelector(".details-name")?.textContent).toBe(
    file.name,
  );
  await act(() =>
    row.dispatchEvent(
      new MouseEvent("click", { bubbles: true, ctrlKey: true }),
    ),
  );
  expect(inspector.hidden).toBe(false);
  expect(inspector.getAttribute("aria-label")).toBe("Current folder details");
});

it("shows only one rescan action when the selected location matches the saved root", async () => {
  await mountHistory();
  await act(() => locationTarget("/fixture").click());
  await flush();
  expectHistoricalStatus();
  expect(container.querySelector(".historical-rescan")).toBeNull();
  expect(container.querySelectorAll(".rescan-action")).toHaveLength(1);
  expect(rescanAction().getAttribute("aria-label")).toBe("Rescan Home");
  await click("Rescan Home");
  expect(
    mock.invoke.mock.calls.some(
      ([command, args]) =>
        command === "rescan_saved_scan" && args.scanId === "historical",
    ),
  ).toBe(true);
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "start_scan"),
  ).toBe(false);
});

it("shows incremental reuse and safe full-scan fallback reasons", async () => {
  mock.invoke.mockImplementationOnce(async () => "scan-1");
  await click("Scan location");
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary(),
      scanMode: "incremental",
      scanPhase: "done",
      reusedEntries: 1234,
      rescannedDirectories: 2,
    }),
  );
  await flush();
  expect(container.textContent).toContain("1,234 metadata entries reused");
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary(),
      scanMode: "full",
      scanPhase: "done",
      fallbackReason: "Filesystem event journal changed",
      cacheWarning: "Could not keep the rescan cache: disk full",
    }),
  );
  await flush();
  const information = container.querySelector<HTMLElement>(
    ".scan-details-popover",
  )!;
  expect(information.hidden).toBe(true);
  expect(information.textContent).toContain(
    "Could not keep the rescan cache: disk full",
  );
  expect(container.querySelectorAll(".scan-note")).toHaveLength(
    information.querySelectorAll(".scan-note").length,
  );
  await act(async () => {
    container.querySelector<HTMLElement>(".scan-totals")!.click();
  });
  expect(information.hidden).toBe(false);
  expect(container.textContent).toContain(
    "Could not keep the rescan cache: disk full",
  );
});

it("blocks session switching and cancellation during cache finalization", async () => {
  await click("Scan location");
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary("scan-1", "running"),
      scanMode: "incremental",
      scanPhase: "caching",
      reusedEntries: 42,
    }),
  );
  await flush();
  expect(container.textContent).toContain("Finishing scan…");
  const buttons = Array.from(container.querySelectorAll("button"));
  expect(buttons.find((b) => b.textContent?.trim() === "Stop")?.disabled).toBe(
    true,
  );
  expect(buttons.find((b) => b.classList.contains("primary"))?.disabled).toBe(
    true,
  );
});

it("loads the published incremental root after a same-count placeholder lookup fails", async () => {
  const original = mock.invoke.getMockImplementation()!;
  let published = false;
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "scan_summary")
        return {
          ...summary("scan-1", "running"),
          scanMode: "incremental",
          scanPhase: "scanning",
        };
      if (command === "get_node" && !published) throw new Error("Unknown node");
      return original(command, args);
    },
  );
  await click("Scan location");
  expect(container.querySelector("[role=treeitem]")).toBeNull();
  published = true;
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary(),
      scanMode: "incremental",
      scanPhase: "done",
    }),
  );
  await flush();
  expect(container.querySelector("[role=treeitem]")?.textContent).toContain(
    "large.dat",
  );
});

it("compares the finished Current scan with a saved scan of its location without saving it", async () => {
  const original = mock.invoke.getMockImplementation()!;
  let entries = [{ ...savedEntry, id: "elsewhere", rootPath: "/other" }];
  mock.invoke.mockImplementation((command, args) => {
    if (command === "list_saved_scans")
      return Promise.resolve({ entries, lastOpenedId: null });
    return original(command, args);
  });
  const compareButton = () =>
    container.querySelector<HTMLButtonElement>(
      '[aria-label="Compare saved scans"]',
    )!;
  await act(async () => {
    root.unmount();
    root = createRoot(container);
    root.render(App);
  });
  await flush();
  await click("Scan location");
  await act(async () =>
    mock.channels[0].onmessage(summary("scan-1", "running")),
  );
  await flush();
  expect(compareButton().disabled).toBe(true);
  await act(async () => mock.channels[0].onmessage(summary()));
  await flush();
  // A finished Current scan needs a saved scan of the same location.
  expect(compareButton().disabled).toBe(true);
  entries = [savedEntry, ...entries];
  await act(async () => {
    root.unmount();
    root = createRoot(container);
    root.render(App);
  });
  await flush();
  await click("Scan location");
  await act(async () => mock.channels.at(-1)!.onmessage(summary()));
  await flush();
  expect(compareButton().disabled).toBe(false);
  await click("Compare saved scans");
  expect(container.textContent).toContain(
    "Comparing the Current scan with a saved scan",
  );
  await click("Compare scans");
  const call = mock.invoke.mock.calls.find(
    ([c]) => c === "compare_current_scan",
  )!;
  expect(call[1]).toMatchObject({
    scanId: "scan-1",
    savedId: savedEntry.id,
    currentAfter: true,
  });
  await click("Swap scans");
  await click("Compare scans");
  expect(
    mock.invoke.mock.calls
      .filter(([c]) => c === "compare_current_scan")
      .at(-1)![1],
  ).toMatchObject({
    scanId: "scan-1",
    savedId: savedEntry.id,
    currentAfter: false,
  });
  expect(
    mock.invoke.mock.calls.some(([c]) =>
      ["save_scan", "compare_saved_scans"].includes(c),
    ),
  ).toBe(false);
});

it("compares saved scans with progress while preserving the displayed session", async () => {
  const original = mock.invoke.getMockImplementation()!;
  const newer = {
    ...savedEntry,
    id: "saved-2",
    completedAtMs: 4000,
    savedAtMs: 5000,
  };
  let finish!: (value: unknown) => void;
  mock.invoke.mockImplementation((command, args) => {
    if (command === "list_saved_scans")
      return Promise.resolve({
        entries: [newer, savedEntry],
        lastOpenedId: savedEntry.id,
      });
    if (command === "open_saved_scan")
      return Promise.resolve({
        ...summary("historical"),
        origin: "saved",
        savedId: savedEntry.id,
      });
    if (command === "compare_saved_scans")
      return new Promise((resolve) => {
        finish = resolve;
      });
    return original(command, args);
  });
  await act(async () => {
    root.unmount();
    root = createRoot(container);
    root.render(App);
  });
  await flush();
  await click("Compare saved scans");
  expect(container.querySelector("main header")).toBeNull();
  expect(container.querySelector(".explorer")).toBeNull();
  await click("Compare scans");
  const call = mock.invoke.mock.calls.find(
    ([c]) => c === "compare_saved_scans",
  )!;
  expect(call[1].beforeId).toBe(savedEntry.id);
  expect(call[1].afterId).toBe(newer.id);
  const channel = call[1].onProgress;
  await act(async () =>
    channel.onmessage({
      operation: "compare",
      phase: "readingAfter",
      bytes: 1024,
    }),
  );
  expect(container.querySelector("[role=status]")?.textContent).toContain(
    "1 KiB (1.02 KB)",
  );
  expect(
    [...container.querySelectorAll(".saved-entry button")].every(
      (b) => (b as HTMLButtonElement).disabled,
    ),
  ).toBe(true);
  await act(async () =>
    finish({
      beforeId: savedEntry.id,
      afterId: newer.id,
      rootPath: "/fixture",
      before: {
        entries: 2,
        logicalBytes: 100,
        allocatedBytes: 100,
        incomplete: false,
        errors: 0,
        excluded: 0,
      },
      after: {
        entries: 2,
        logicalBytes: 200,
        allocatedBytes: 200,
        incomplete: false,
        errors: 0,
        excluded: 0,
      },
      logicalDelta: 100,
      allocatedDelta: 100,
      added: 0,
      removed: 0,
      changed: 1,
      unchanged: 0,
      warnings: ["Recorded metadata only"],
      rankings: [],
    }),
  );
  await flush();
  expect(container.textContent).toContain("Comparing two past scans");
  expect(container.textContent).toContain("Recorded metadata only");
  expect(
    mock.invoke.mock.calls.filter(([c]) => c === "open_saved_scan"),
  ).toHaveLength(1);
  mock.invoke.mockImplementationOnce(async () => {
    throw new Error("Comparison archive is corrupt");
  });
  await click("Compare scans");
  expect(container.querySelector('[role="dialog"] h2')?.textContent).toContain(
    "Couldn’t compare the scans",
  );
  expect(
    container.querySelector("[role=dialog] details")?.textContent,
  ).toContain("Comparison archive is corrupt");
  expect(container.textContent).toContain("Recorded metadata only");
  expect(
    container
      .querySelector(".comparison-panel button")
      ?.hasAttribute("disabled"),
  ).toBe(false);
  expect(
    mock.invoke.mock.calls.some(([c]) =>
      ["prepare_trash", "commit_trash", "reveal_node"].includes(c),
    ),
  ).toBe(false);
  await click("Close comparison");
  expect(container.querySelector("main header")).toBeTruthy();
  expectHistoricalStatus();
});

it("traps Trash dialog focus, cancels with Escape and restores the opener", async () => {
  const original = mock.invoke.getMockImplementation()!;
  let prepared!: (value: unknown) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "prepare_trash"
      ? new Promise((resolve) => {
          prepared = resolve;
        })
      : original(command, args),
  );
  await click("Scan location");
  await act(async () =>
    (container.querySelector('[role="treeitem"]') as HTMLElement).click(),
  );
  const opener = container.querySelector<HTMLElement>('[role="treeitem"]')!;
  opener.tabIndex = 0;
  opener.focus();
  await act(async () => menuItem("trash").click());
  // Real browsers can blur the focused row before the asynchronous preview arrives.
  container.tabIndex = -1;
  container.focus();
  expect(document.activeElement).toBe(container);
  await act(async () => prepared({ token: "approved", items: [file] }));
  await flush();
  const dialog = container.querySelector<HTMLElement>('[role="dialog"]')!;
  const buttons = [...dialog.querySelectorAll<HTMLButtonElement>("button")];
  expect(document.activeElement).toBe(buttons[0]);
  const reviewedItems = dialog.querySelector<HTMLElement>(".trash-items")!;
  // jsdom has no native Tab movement; browser smoke covers reaching the list.
  reviewedItems.focus();
  await act(async () =>
    reviewedItems.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(document.activeElement).toBe(buttons.at(-1));
  await act(async () =>
    buttons.at(-1)!.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(document.activeElement).toBe(reviewedItems);
  await act(async () =>
    reviewedItems.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(document.activeElement).toBe(opener);
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "commit_trash"),
  ).toBe(false);
});

it("keeps pending Trash confirmation open when Escape is pressed", async () => {
  const original = mock.invoke.getMockImplementation()!;
  let finish!: (value: unknown) => void;
  mock.invoke.mockImplementation((command, args) =>
    command === "commit_trash"
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : original(command, args),
  );
  await click("Scan location");
  await act(async () =>
    (container.querySelector('[role="treeitem"]') as HTMLElement).click(),
  );
  await click("Move to Trash");
  await act(async () =>
    container.querySelector<HTMLButtonElement>(".modal .danger")!.click(),
  );
  const dialog = container.querySelector<HTMLElement>('[role="dialog"]')!;
  await act(async () =>
    dialog.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(container.querySelector('[role="dialog"]')).toBe(dialog);
  expect(dialog.textContent).toContain("Moving…");
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "commit_trash"),
  ).toHaveLength(1);
  await act(async () => finish({ moved: 1, failures: [] }));
  await flush();
  // The confirmation is replaced by the cleanup result modal.
  expect(container.querySelector(".trash-modal")).toBeNull();
  expect(container.querySelector(".cleanup-modal")).not.toBeNull();
});

it("keeps the inspector size following the selected unit when switching metrics", async () => {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "get_node" && args.nodeId === 0)
      return { ...rootNode, allocatedBytes: 128, logicalBytes: 64 };
    return original(command, args);
  });
  await click("Scan location");
  const size = () =>
    container
      .querySelector<HTMLElement>(".details-size")!
      .textContent?.replace(/\s+/g, " ")
      .trim();
  expect(size()).toBe("128 B on disk");
  await click("Logical");
  expect(size()).toBe("64 B logical");
});

it("shows combined sizes and action scope for a multi-selection instead of the last file", async () => {
  const original = mock.invoke.getMockImplementation()!;
  const second = {
    ...file,
    id: 2,
    name: "second.dat",
    path: "/fixture/second.dat",
    allocatedBytes: 64,
    logicalBytes: 32,
  };
  mock.invoke.mockImplementation(async (command, args) =>
    command === "get_children"
      ? { items: [file, second], total: 2 }
      : original(command, args),
  );
  await click("Scan location");
  const rows = container.querySelectorAll<HTMLElement>('[role="treeitem"]');
  await act(() => rows[0].click());
  await act(() =>
    rows[1].dispatchEvent(
      new MouseEvent("click", { bubbles: true, ctrlKey: true }),
    ),
  );
  await flush();
  const inspector = container.querySelector(".inspector")!;
  expect(inspector.querySelector(".details-name")?.textContent).toBe(
    "2 items selected",
  );
  expect(inspector.querySelector(".details-size")).toBeNull();
  expect(inspector.textContent?.replace(/\s+/g, " ")).toContain(
    "apply to all 2 selected items",
  );
  expect(
    inspector.querySelector<HTMLButtonElement>(
      '[aria-label="Reveal in Finder"]',
    )!.disabled,
  ).toBe(true);
  expect(inspector.querySelector(".danger")).toBeNull();
  expect(menuItem("trash").enabled).toBe(true);
});


async function scanWithWarnings() {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "scan_summary")
      return {
        ...summary(String(args.scanId)),
        accountingWarnings: ["clone metadata changed during validation"],
      };
    return original(command, args);
  });
  await click("Scan location");
}
it("keeps scan warnings quiet: no badge or dialog, one line in Summary", async () => {
  await scanWithWarnings();
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(
    [...container.querySelectorAll("button")].some((b) =>
      /\d+ warnings?$/.test(b.textContent?.trim() ?? ""),
    ),
  ).toBe(false);
  const popover = container.querySelector(".scan-details-popover")!;
  expect(popover.textContent?.replace(/\s+/g, " ")).toContain(
    "Some files changed during the scan, so a few sizes may be slightly off.",
  );
  expect(container.textContent).not.toContain(
    "clone metadata changed during validation",
  );
});
it("shows no warning line for a scan without warnings", async () => {
  await click("Scan location");
  expect(
    container.querySelector(".scan-details-popover")?.textContent,
  ).not.toContain("Some files changed during the scan");
});
it("scans the location on screen from the failed scan modal, not the sidebar selection", async () => {
  await click("Scan location");
  await act(async () => locationTarget("Other").click());
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary("scan-1", "failed"),
      message: "Disk vanished",
    }),
  );
  await act(() => failureButton("Scan again")!.click());
  await flush();
  const starts = mock.invoke.mock.calls.filter(([c]) => c === "start_scan");
  expect(starts).toHaveLength(2);
  expect(starts[1][1].targetId).toBe("home");
});
