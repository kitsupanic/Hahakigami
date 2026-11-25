// @vitest-environment jsdom
import { act, createTestRoot, type Root, fullEdition } from "./testUtils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App.svelte";
import type createApp from "./createApp.svelte";
import type { Target } from "./types";

const mock = vi.hoisted(() => ({
  invoke: vi.fn(),
  native: true,
  app: null as ReturnType<typeof createApp> | null,
}));
vi.mock("./createApp.svelte", async (original) => {
  const module = await original<typeof import("./createApp.svelte")>();
  return { default: () => (mock.app = module.default()) };
});
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => mock.native,
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
const folder: Target = {
  id: "picked",
  name: "Photos",
  path: "/fixture/Photos",
  totalBytes: null,
  freeBytes: null,
};
const home: Target = {
  id: "home",
  name: "fixture",
  path: "/Users/fixture",
  totalBytes: null,
  freeBytes: null,
  home: true,
};
const app = () => mock.app!;
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
let container: HTMLDivElement, root: Root;
const calls = (command: string) =>
  mock.invoke.mock.calls.filter(([name]) => name === command);
// The welcome card is the "Choose folder" control; it has no aria-label, its text names it.
const button = (name: string) =>
  container.querySelector<HTMLButtonElement>(
    name === "Choose folder"
      ? "button.welcome-target"
      : name === "Scan location"
        ? "button.welcome-scan"
        : `button[aria-label="${name}"]`,
  )!;
async function mount() {
  await act(() => root.render(App));
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
async function click(name: string) {
  await act(() => button(name).click());
}
function response(command: string, result: unknown | (() => unknown)) {
  const original = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((name, args) =>
    name === command
      ? typeof result === "function"
        ? result()
        : Promise.resolve(result)
      : original(name, args),
  );
}
beforeEach(() => {
  mock.native = true;
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "get_edition":
        return fullEdition;
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return {
          status: "on",
          scanAnyway: false,
          appPath: "/Applications/Fixture.app",
        };
      case "discover_targets":
        return [disk];
      case "list_cleanup_queue":
        return { entries: [], revision: 0, warning: null, blocked: false };
      case "choose_folder":
        return folder;
      case "start_scan":
        return new Promise(() => {});
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

it("offers a central explicit scan of the selected disk", async () => {
  await mount();
  const scan = button("Scan location");
  expect(scan.closest(".welcome")).not.toBeNull();
  expect(scan.textContent).toContain("Scan Startup disk");
  expect(calls("start_scan")).toHaveLength(0);
  await click("Scan location");
  expect(calls("start_scan")[0][1]).toMatchObject({ targetId: disk.id });
});
it("selects a native folder in the welcome card and waits for an explicit scan", async () => {
  await mount();
  await click("Choose folder");
  expect(app().target).toEqual(folder);
  expect(button("Choose folder").title).toBe(folder.path);
  expect(button("Scan location").textContent).toContain("Scan Photos");
  // The visible text is the accessible name (label in name).
  expect(button("Scan location").hasAttribute("aria-label")).toBe(false);
  expect(calls("start_scan")).toHaveLength(0);
  await click("Scan location");
  expect(calls("start_scan")[0][1]).toMatchObject({ targetId: folder.id });
});
it("deduplicates registered targets when the same folder is chosen again", async () => {
  await mount();
  await click("Choose folder");
  await click("Choose folder");
  expect(calls("choose_folder")).toHaveLength(2);
  expect(
    app().targets.filter((target) => target.id === folder.id),
  ).toHaveLength(1);
  expect(button("Scan location").textContent).toContain("Scan Photos");
});
it("keeps selection and feedback on cancellation and reports a current picker failure", async () => {
  await mount();
  await click("Choose folder");
  await act(() => app().setError("Previous feedback"));
  response("choose_folder", null);
  await click("Choose folder");
  expect(app().target).toEqual(folder);
  expect(app().summary).toBeNull();
  expect(app().error).toBe("Previous feedback");
  response("choose_folder", () =>
    Promise.reject(new Error("Picker unavailable")),
  );
  await click("Choose folder");
  expect(app().target).toEqual(folder);
  expect(container.querySelector('[role="dialog"] h2')?.textContent).toContain(
    "Couldn’t choose the folder",
  );
  expect(
    container.querySelector("[role=dialog] details")?.textContent,
  ).toContain("Picker unavailable");
  expect(app().choosingFolder).toBe(false);
});
it("blocks the picker, location changes, and scanning during a native picker", async () => {
  const picking = deferred<Target | null>();
  response("choose_folder", () => picking.promise);
  await mount();
  await click("Choose folder");
  expect(app().choosingFolder).toBe(true);
  for (const name of ["Choose folder", "Scan location"])
    expect(button(name).disabled).toBe(true);
  await act(() => {
    void app().chooseFolder();
    void app().start();
  });
  expect(calls("choose_folder")).toHaveLength(1);
  expect(calls("start_scan")).toHaveLength(0);
  await act(() => picking.resolve(folder));
  expect(app().choosingFolder).toBe(false);
  expect(button("Choose folder").disabled).toBe(false);
});
it("waits for discovery before opening a picker so discovery cannot replace a picked target", async () => {
  const discovery = deferred<Target[]>();
  response("discover_targets", () => discovery.promise);
  await mount();
  expect(app().targetsLoading).toBe(true);
  expect(button("Choose folder").disabled).toBe(true);
  await act(() => app().chooseFolder());
  expect(calls("choose_folder")).toHaveLength(0);
  await act(() => discovery.resolve([disk]));
  expect(app().targetsLoading).toBe(false);
  await click("Choose folder");
  expect(app().target).toEqual(folder);
});
it("allows choosing a folder after discovery fails with no locations", async () => {
  response("discover_targets", () =>
    Promise.reject(new Error("Discovery unavailable")),
  );
  await mount();
  expect(app().targetsLoading).toBe(false);
  expect(button("Scan location").disabled).toBe(true);
  expect(button("Choose folder").disabled).toBe(false);
  await click("Choose folder");
  expect(app().target).toEqual(folder);
});
it("opens the Full Disk Access dialog instead of scanning while access is off", async () => {
  response("full_disk_access_status", {
    status: "off",
    scanAnyway: false,
    appPath: "/Applications/Fixture.app",
  });
  await mount();
  expect(app().accessOpen).toBe(false);
  await click("Scan location");
  expect(app().accessOpen).toBe(true);
  expect(calls("start_scan")).toHaveLength(0);
});
it("does not gate choosing a folder on Full Disk Access", async () => {
  response("full_disk_access_status", {
    status: "off",
    scanAnyway: false,
    appPath: "/Applications/Fixture.app",
  });
  await mount();
  await click("Choose folder");
  expect(app().accessOpen).toBe(false);
  expect(calls("choose_folder")).toHaveLength(1);
});
it("disables scan and folder controls in browser preview", async () => {
  mock.native = false;
  await mount();
  for (const name of ["Scan location", "Choose folder"])
    expect(button(name).disabled).toBe(true);
  await act(() => app().chooseFolder());
  expect(calls("choose_folder")).toHaveLength(0);
  expect(app().targetsLoading).toBe(false);
});
it("ignores a native picker completion after teardown", async () => {
  const picking = deferred<Target | null>();
  response("choose_folder", () => picking.promise);
  await mount();
  await click("Choose folder");
  const old = app();
  await act(() => root.unmount());
  await act(() => picking.resolve(folder));
  expect(old.target).toEqual(disk);
});

it("rejects picker requests while the access dialog is open", async () => {
  await mount();
  await act(() => app().openAccess());
  await act(() => app().chooseFolder());
  expect(calls("choose_folder")).toHaveLength(0);
});
it.each(["scan", "navigation", "target"])(
  "handles a picker completion after the current %s changes",
  async (context) => {
    const picking = deferred<Target | null>();
    response("choose_folder", () => picking.promise);
    await mount();
    let pending!: Promise<Target | null | void>;
    await act(() => {
      pending = app().chooseFolder();
    });
    await act(() => {
      if (context === "scan") app().activeId.current = "newer";
      if (context === "navigation") app().navigation.current++;
      if (context === "target") app().setTarget({ ...disk, id: "newer" });
    });
    await act(async () => {
      picking.resolve(folder);
      await pending;
    });
    // Neither another scan nor a disk refresh replacing the target may drop the chosen folder.
    expect(app().target?.id).toBe(folder.id);
    expect(app().choosingFolder).toBe(false);
  },
);

it("selects the startup disk by default so the first scan is one click", async () => {
  response("discover_targets", [home, disk]);
  await mount();
  expect(app().target).toEqual(disk);
  const scan = button("Scan location");
  expect(scan.disabled).toBe(false);
  expect(scan.textContent).toContain("Scan Startup disk");
  expect(container.querySelector(".welcome-note")?.textContent).toBe(
    "Nothing leaves your Mac.",
  );
  await click("Scan location");
  expect(calls("start_scan")[0][1]).toMatchObject({ targetId: disk.id });
});
it("makes the target card a button that names the target and opens the chooser", async () => {
  response("discover_targets", [home, disk]);
  await mount();
  const card = button("Choose folder");
  expect(card.tagName).toBe("BUTTON");
  expect(card.textContent).toContain("Startup disk");
  expect(card.textContent).toContain("Choose another location");
  expect(button("Scan location").getAttribute("aria-describedby")).toBe(
    "welcome-note",
  );
  await click("Choose folder");
  expect(calls("choose_folder")).toHaveLength(1);
  expect(app().target).toEqual(folder);
});
it("shows a visible hint instead of a silently disabled scan when no location exists", async () => {
  response("discover_targets", []);
  await mount();
  expect(app().target).toBeNull();
  expect(button("Scan location").disabled).toBe(true);
  expect(container.querySelector(".welcome-note")?.textContent).toBe(
    "Choose a location to scan.",
  );
  expect(button("Choose folder").textContent).toContain("Choose a location");
});
