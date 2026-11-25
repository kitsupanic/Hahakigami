// @vitest-environment jsdom
import { act, createTestRoot, fullEdition, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { Edition, FullDiskAccess, Target } from "./types";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  app: null as ReturnType<typeof createApp> | null,
}));
// The Full Disk Access card only renders under the macOS titlebar.
vi.hoisted(() =>
  Object.defineProperty(navigator, "platform", {
    value: "MacIntel",
    configurable: true,
  }),
);
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
const lite: Edition = {
  productName: "Hahakigami Lite",
  automationScans: false,
  scanTargetsRequireAllowedLocation: true,
};
const disk: Target = {
  id: "disk",
  name: "Startup disk",
  path: "/",
  totalBytes: 1000,
  freeBytes: 500,
  volume: true,
};
let container: HTMLDivElement, root: Root;
let edition: Edition | Error;
let access: FullDiskAccess;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
async function mount() {
  await act(() => root.render(App));
  await act(async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  });
}
const brand = () => container.querySelector(".brand div")?.textContent;
beforeEach(() => {
  edition = fullEdition;
  access = { status: "on", scanAnyway: false, appPath: "/Fixture.app" };
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "get_edition":
        if (edition instanceof Error) throw edition;
        return edition;
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return access;
      case "discover_targets":
        return [disk];
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
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

it("reads the edition answers once through one command", async () => {
  await mount();
  expect(calls("get_edition")).toHaveLength(1);
});
it("keeps the Full edition's name where the app shows it", async () => {
  await mount();
  expect(brand()).toBe("Hahakigami");
  expect(container.querySelector(".window-toolbar-fallback strong")?.textContent).toBe(
    "Hahakigami",
  );
});
it("names the Lite edition where the app shows its name", async () => {
  edition = lite;
  await mount();
  expect(brand()).toBe("Hahakigami Lite");
  expect(container.querySelector(".window-toolbar-fallback strong")?.textContent).toBe(
    "Hahakigami Lite",
  );
});
it("names the edition in the Full Disk Access card", async () => {
  edition = lite;
  access = { status: "off", scanAnyway: false, appPath: "/Fixture.app" };
  await mount();
  await act(() => mock.app!.openAccess());
  await act(async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  });
  const text = container.textContent ?? "";
  expect(text).toContain("Turn on Hahakigami Lite in");
  expect(text).not.toMatch(/Hahakigami(?! Lite)\b/);
});
it("shows no Automation scan, CLI or MCP wording when they are unavailable", async () => {
  edition = lite;
  await mount();
  expect(container.textContent).not.toMatch(/automation|\bMCP\b|\bCLI\b|command.line/i);
});
it("shows no product name when the answers cannot be read", async () => {
  edition = new Error("unavailable");
  await mount();
  expect(brand()).toBe("");
  expect(container.textContent).not.toContain("Hahakigami");
});
