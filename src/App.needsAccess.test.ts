// @vitest-environment jsdom
import { act, createTestRoot, type Root } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { Target } from "./types";

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
const disk: Target = {
  id: "disk",
  name: "Startup disk",
  path: "/",
  totalBytes: 1000,
  freeBytes: 500,
  volume: true,
};
let container: HTMLDivElement, root: Root;
const app = () => mock.app!;
const flush = () =>
  act(async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
  });
beforeEach(() => {
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "get_edition":
        return {
          productName: "Hahakigami Lite",
          automationScans: false,
          scanTargetsRequireAllowedLocation: true,
        };
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return { status: "on", scanAnyway: false, appPath: "/Fixture.app" };
      case "discover_targets":
        return [disk];
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
      case "start_scan":
        throw "NEEDS_ACCESS: Allow access to / before scanning it.";
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
async function mount() {
  await act(() => root.render(App));
  await flush();
}

it("explains a scan refused for lack of access in plain words, not as a failure or a Full Disk Access problem", async () => {
  await mount();
  await act(() => app().start(disk));
  await flush();
  const dialog = container.querySelector(".failure-results")!;
  expect(dialog.querySelector("h2")?.textContent).toBe("Allow access first");
  expect(dialog.querySelector(".failure-reason")?.textContent).toMatch(
    /Startup disk/,
  );
  expect(container.textContent).not.toContain("NEEDS_ACCESS");
  expect(container.textContent).not.toContain("Full Disk Access");
  expect(container.textContent).not.toContain("Scan failed");
});
it("keeps the disk selectable and scannable again after the refusal", async () => {
  await mount();
  await act(() => app().start(disk));
  await flush();
  await act(() => app().setError(""));
  expect(container.querySelector(".failure-results")).toBeNull();
  expect(app().can.rescan).toBe(true);
});
