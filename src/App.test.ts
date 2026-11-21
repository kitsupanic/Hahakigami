// @vitest-environment jsdom
import { act, createTestRoot as createRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
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
  const b = Array.from(container.querySelectorAll("button")).find(
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
const rescanAction = () =>
  container.querySelector<HTMLButtonElement>("header .rescan-action")!;
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
  },
});
async function chooseFolderFromSidebar() {
  await act(() =>
    container.querySelector<HTMLButtonElement>("button.scan-folder-nav")!.click(),
  );
  await flush();
}
beforeEach(async () => {
  mock.channels.length = 0;
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      switch (command) {
        case "get_explorer_preferences":
          return { sort: "sizeDescending", warning: null };
        case "set_explorer_sort":
          return { sort: args.sort, warning: null };
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
it("lists the Display rows as Theme, Appearance", async () => {
  await remountWithAccess("on");
  await click("Settings");
  const rows = [...container.querySelectorAll(".settings-row > span")].map(
    (row) => row.textContent?.trim(),
  );
  expect(rows).toEqual(["Theme", "Appearance"]);
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
  await chooseFolderFromSidebar();
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

const failureDialog = () =>
  container.querySelector<HTMLElement>('[role="dialog"]');
// jsdom has no `inert` reflection, so read the property Svelte sets.
const inert = (selector: string) =>
  (container.querySelector(selector) as HTMLElement & { inert: boolean }).inert;
const failureButton = (label: string) =>
  [...(failureDialog()?.querySelectorAll("button") ?? [])].find(
    (b) => b.textContent?.trim() === label,
  ) as HTMLButtonElement | undefined;
const toast = () => container.querySelector<HTMLElement>(".toast");
const toastButton = (label: string) =>
  [...(toast()?.querySelectorAll("button") ?? [])].find(
    (b) =>
      b.textContent?.trim() === label || b.getAttribute("aria-label") === label,
  ) as HTMLButtonElement;
it("opens a modal for a failed folder pick, with the raw error only under Technical details", async () => {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) => {
    if (command === "choose_folder") throw new Error("Picker offline");
    return original(command, args);
  });
  await chooseFolderFromSidebar();
  const dialog = failureDialog()!;
  expect(dialog.querySelector("h2")?.textContent).toContain(
    "Couldn’t choose the folder",
  );
  const details = dialog.querySelector("details")!;
  expect(details.textContent).toContain("Picker offline");
  expect(dialog.textContent?.replace(details.textContent!, "")).not.toContain(
    "Picker offline",
  );
  expect(container.querySelector("main .error")).toBeNull();
  expect(toast()).toBeNull();
  expect(inert("main")).toBe(true);
  await act(() => failureButton("Close")!.click());
  expect(failureDialog()).toBeNull();
  expect(inert("main")).toBe(false);
});
// Each cancelled scan raises one "Scan cancelled" toast.
async function cancelledScan() {
  await act(() => mock.app!.start());
  await flush();
  await act(async () =>
    mock.channels.at(-1)!.onmessage(summary("scan-1", "cancelled")),
  );
}
it("announces a toast politely without moving focus or adding a card above the map", async () => {
  await click("Scan location");
  const focused = document.activeElement;
  await cancelledScan();
  expect(toast()?.textContent).toContain("Scan cancelled");
  const region = toast()!.closest("[aria-live]")!;
  expect(region.getAttribute("aria-live")).toBe("polite");
  expect(region.getAttribute("role")).toBe("status");
  expect(container.contains(document.activeElement)).toBe(
    container.contains(focused),
  );
  expect(toast()!.contains(document.activeElement)).toBe(false);
  expect(container.querySelector(".scan-note")).toBeNull();
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
it("shows a start error only after the scan failure is dismissed", async () => {
  await click("Scan location");
  await act(async () =>
    mock.channels[0].onmessage({
      ...summary("scan-1", "failed"),
      message: "Disk vanished",
    }),
  );
  await act(() => mock.app!.setError("Start broke"));
  expect(container.querySelectorAll('[role="dialog"]')).toHaveLength(1);
  expect(failureDialog()!.querySelector("h2")?.textContent).toContain(
    "Scan failed",
  );
  await act(() => failureButton("Close")!.click());
  expect(container.querySelectorAll('[role="dialog"]')).toHaveLength(1);
  expect(failureDialog()!.textContent).toContain("Start broke");
});
it("keeps a toast while its Dismiss button has focus after the pointer leaves", async () => {
  await click("Scan location");
  vi.useFakeTimers();
  try {
    await cancelledScan();
    await act(() => {
      toast()!.dispatchEvent(new MouseEvent("mouseenter"));
      toast()!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    await act(() =>
      toastButton("Dismiss").dispatchEvent(
        new FocusEvent("focusin", { bubbles: true }),
      ),
    );
    await act(() => {
      toast()!.dispatchEvent(new MouseEvent("mouseleave"));
      toast()!.dispatchEvent(new MouseEvent("mouseout", { bubbles: true }));
    });
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(toast()).not.toBeNull();
    await act(() =>
      toastButton("Dismiss").dispatchEvent(
        new FocusEvent("focusout", { bubbles: true }),
      ),
    );
    await act(() => vi.advanceTimersByTimeAsync(4100));
    expect(toast()).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});
it("fades a toast after about four seconds, unless hovered or focused", async () => {
  await click("Scan location");
  vi.useFakeTimers();
  try {
    await cancelledScan();
    await act(() => vi.advanceTimersByTimeAsync(3900));
    expect(toast()).not.toBeNull();
    await act(() => vi.advanceTimersByTimeAsync(300));
    expect(toast()).toBeNull();
    await cancelledScan();
    await act(() =>
      toast()!.dispatchEvent(new MouseEvent("mouseenter")),
    );
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(toast()).not.toBeNull();
    await act(() =>
      toast()!.dispatchEvent(new MouseEvent("mouseleave")),
    );
    await act(() => vi.advanceTimersByTimeAsync(4100));
    expect(toast()).toBeNull();
    await cancelledScan();
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
    // A toast that appears under a resting pointer gets no mouseenter, but is hovered.
    const original = Element.prototype.matches;
    const matches = vi
      .spyOn(Element.prototype, "matches")
      .mockImplementation(function (this: Element, selector: string) {
        return selector === ":hover"
          ? this.classList.contains("toast")
          : original.call(this, selector);
      });
    try {
      await cancelledScan();
    } finally {
      matches.mockRestore();
    }
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(toast()).not.toBeNull();
    await act(() =>
      toast()!.dispatchEvent(new MouseEvent("mouseleave")),
    );
    await act(() => vi.advanceTimersByTimeAsync(4100));
    expect(toast()).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});
it("dismisses a toast early and lets a new toast replace the current one", async () => {
  await click("Scan location");
  await cancelledScan();
  await act(() => toastButton("Dismiss").click());
  expect(toast()).toBeNull();
  await cancelledScan();
  const first = toast();
  expect(first?.textContent).toContain("Scan cancelled");
  await cancelledScan();
  expect(container.querySelectorAll(".toast")).toHaveLength(1);
  expect(toast()).not.toBe(first);
});
it("the inspector returns to current-folder context when an explicit selection clears", async () => {
  await click("Scan location");
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

it("shows only one rescan action when the selected location matches the scanned root", async () => {
  await click("Scan location");
  await act(() => locationTarget("/fixture").click());
  await flush();
  expect(container.querySelectorAll(".rescan-action")).toHaveLength(1);
  expect(rescanAction().getAttribute("aria-label")).toBe("Rescan Home");
  const starts = () =>
    mock.invoke.mock.calls.filter(([c]) => c === "start_scan").length;
  const before = starts();
  await click("Rescan Home");
  expect(starts()).toBe(before + 1);
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

it("shows combined sizes and action scope for a multi-selection instead of the last file", async () => {
  const original = mock.invoke.getMockImplementation()!;
  const second = {
    ...file,
    id: 2,
    name: "second.dat",
    path: "/fixture/second.dat",
    allocatedBytes: 64,
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
it("scans a folder chosen from the sidebar and leaves the Cleanup queue to show it", async () => {
  const photos = {
    id: "photos",
    path: "/Users/test/Photos",
    name: "Photos",
    totalBytes: null,
    freeBytes: null,
    volume: false,
  };
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation(async (command, args) =>
    command === "choose_folder" ? photos : original(command, args),
  );
  await act(() => mock.app!.showQueue(true));
  const before = calls("start_scan").length;
  await act(() =>
    container
      .querySelector<HTMLButtonElement>("button.scan-folder-nav")!
      .click(),
  );
  await flush();
  expect(calls("start_scan")).toHaveLength(before + 1);
  expect(mock.app!.target?.path).toBe(photos.path);
  expect(mock.app!.queueOpen).toBe(false);
});
