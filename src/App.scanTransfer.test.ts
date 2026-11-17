// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, createTestRoot, type Root } from "./testUtils";
import Harness from "./QueueRemovalHarness.svelte";
import type createApp from "./createApp.svelte";
import type { LibraryProgress, SavedEntry, ScanSummary } from "./types";

const mock = vi.hoisted(() => ({ invoke: vi.fn(), native: true }));
vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => mock.native,
  invoke: mock.invoke,
  Channel: class {
    onmessage = (_: LibraryProgress) => {};
  },
}));
let app: ReturnType<typeof createApp>, root: Root, container: HTMLDivElement;
const original: SavedEntry = {
  id: "original",
  name: "Original scan",
  rootPath: "/original",
  startedAtMs: 0,
  completedAtMs: 1000,
  savedAtMs: 2000,
  entries: 2,
  logicalBytes: 100,
  allocatedBytes: 200,
  incomplete: false,
  errors: 0,
  excluded: 0,
  fileBytes: 100,
};
const imported: SavedEntry = {
  ...original,
  id: "native-import-id",
  name: "Imported scan",
  rootPath: "/imported/location",
  savedAtMs: 3000,
};
const historical: ScanSummary = {
  scanId: "opened",
  savedId: "original",
  origin: "saved",
  state: "completed",
  rootId: 0,
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries: 2,
  logicalBytes: 100,
  allocatedBytes: 200,
  elapsedMs: 10,
  errors: 0,
};
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
function channel(command: string) {
  const call = mock.invoke.mock.calls
    .filter(([name]) => name === command)
    .at(-1)!;
  return call[1].onProgress as { onmessage: (p: LibraryProgress) => void };
}
function intercept(command: string, value: Promise<unknown>) {
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((name, args) =>
    name === command ? value : standard(name, args),
  );
}
beforeEach(async () => {
  mock.native = true;
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command) => {
    switch (command) {
      case "get_explorer_preferences":
        return { sort: "sizeDescending", warning: null };
      case "full_disk_access_status":
        return { status: "off", scanAnyway: false, appPath: "/Fixture.app" };
      case "list_saved_scans":
        return { entries: [{ ...original }], lastOpenedId: original.id };
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
      case "list_cleanup_queue":
        return { entries: [], revision: 0, blocked: false, warning: null };
      case "open_saved_scan":
        return { ...historical };
      case "get_node":
        return {
          id: 0,
          parentId: null,
          name: "Original",
          path: "/original",
          kind: "directory",
          logicalBytes: 100,
          allocatedBytes: 200,
          childrenCount: 1,
          incomplete: false,
        };
      case "get_map":
        return { nodes: [] };
      case "get_children":
        return { items: [], total: 0 };
      case "import_saved_scan":
        return { ...imported };
      case "export_saved_scan":
        return true;
      default:
        return undefined;
    }
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
  await act(() =>
    root.render(Harness, {
      capture: (value) => {
        app = value;
      },
    }),
  );
  await flush();
});
afterEach(async () => {
  await root.unmount();
  container.remove();
});

it.each(["import", "export"] as const)(
  "keeps %s pending through byte counter resets and native done until the response confirms completion",
  async (operation) => {
    const native = deferred<unknown>();
    const command = `${operation}_saved_scan`;
    intercept(command, native.promise);
    const pending =
      operation === "import" ? app.importScan() : app.exportScan(original.id);
    expect(app.scanTransfer).toMatchObject({
      operation,
      state: "pending",
      phase: "choosing",
      bytes: 0,
      error: null,
    });
    const events = channel(command);
    await act(() =>
      events.onmessage({
        operation,
        phase: operation === "import" ? "reading" : "writing",
        bytes: 8192,
      }),
    );
    expect(app.scanTransfer).toMatchObject({
      state: "pending",
      phase: "processing",
      bytes: 8192,
    });
    await act(() =>
      events.onmessage({ operation, phase: "validating", bytes: 16 }),
    );
    expect(app.scanTransfer?.bytes).toBe(16);
    await act(() => events.onmessage({ operation, phase: "done", bytes: 0 }));
    expect(app.scanTransfer).toMatchObject({
      state: "pending",
      phase: "finishing",
      bytes: 0,
    });
    expect(app.libraryBusy).toBe(true);
    app.dismissScanTransfer();
    expect(app.scanTransfer?.state).toBe("pending");
    native.resolve(operation === "import" ? imported : true);
    await act(() => pending);
    expect(app.scanTransfer).toBeNull();
    expect(app.toast?.message).toContain(
      operation === "import" ? "Imported scan added" : "Scan exported",
    );
    expect(app.libraryProgress).toBeNull();
    expect(app.libraryBusy).toBe(false);
  },
);

it("adds only a validated native import ID without opening it or replacing restored historical results", async () => {
  const displayed = app.summary;
  expect(displayed?.savedId).toBe(original.id);
  expect(app.access?.status).toBe("off");
  await act(() => app.importScan());
  expect(app.scanTransfer).toBeNull();
  expect(app.toast?.message).toContain("Imported scan added");
  expect(app.saved.map((entry) => entry.id)).toEqual([
    imported.id,
    original.id,
  ]);
  expect(app.summary).toBe(displayed);
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "open_saved_scan"),
  ).toEqual([
    ["open_saved_scan", { id: original.id, onProgress: expect.anything() }],
  ]);
  expect(
    mock.invoke.mock.calls.some(([command]) => command === "start_scan"),
  ).toBe(false);
  expect(mock.invoke).toHaveBeenCalledWith("import_saved_scan", {
    onProgress: expect.anything(),
  });
});

it("preserves live results on successful import", async () => {
  const live = { ...historical, origin: "live" as const, savedId: null };
  app.setSummary(live);
  await flush();
  await act(() => app.importScan());
  expect(app.summary).toBe(live);
});

it("accepts native display-size metadata above JavaScript's safe integer range without changing ID authority", async () => {
  const large = {
    ...imported,
    logicalBytes: Number.MAX_SAFE_INTEGER + 1025,
    allocatedBytes: Number.MAX_SAFE_INTEGER + 2049,
  };
  intercept("import_saved_scan", Promise.resolve(large));
  await act(() => app.importScan());
  expect(app.toast?.message).toContain("Imported scan added");
  expect(app.saved[0].id).toBe(imported.id);
  await act(() => app.exportScan(imported.id));
  expect(app.toast?.message).toContain("Scan exported");
  expect(mock.invoke).toHaveBeenCalledWith("export_saved_scan", {
    id: imported.id,
    onProgress: expect.anything(),
  });
});

it("exports by saved ID only and freezes the displayed metadata without inventing a destination", async () => {
  const native = deferred<boolean>();
  intercept("export_saved_scan", native.promise);
  const entry = app.saved[0];
  const pending = app.exportScan(entry.id);
  entry.name = "Changed while exporting";
  app.setSaved([]);
  native.resolve(true);
  await act(() => pending);
  expect(mock.invoke).toHaveBeenCalledWith("export_saved_scan", {
    id: original.id,
    onProgress: expect.anything(),
  });
  expect(app.scanTransfer).toBeNull();
  expect(app.toast?.message).toContain("Scan exported");
  expect(app.summary?.savedId).toBe(original.id);
});

it("can confirm export without a local metadata copy while retaining native ID authority", async () => {
  app.setSaved([]);
  await act(() => app.exportScan("registered-native-id"));
  expect(app.toast?.message).toContain("Scan exported");
  expect(mock.invoke).toHaveBeenCalledWith("export_saved_scan", {
    id: "registered-native-id",
    onProgress: expect.anything(),
  });
});

it.each(["import", "export"] as const)(
  "reports %s cancellation without errors, library changes or automatic retries",
  async (operation) => {
    intercept(
      `${operation}_saved_scan`,
      Promise.resolve(operation === "import" ? null : false),
    );
    const before = app.summary,
      saved = app.saved;
    await act(() =>
      operation === "import" ? app.importScan() : app.exportScan(original.id),
    );
    expect(app.scanTransfer).toBeNull();
    expect(app.toast?.message).toContain(
      operation === "import" ? "Import cancelled" : "Export cancelled",
    );
    expect(app.summary).toBe(before);
    expect(app.saved).toBe(saved);
    expect(app.error).toBe("");
    expect(
      mock.invoke.mock.calls.filter(
        ([command]) => command === `${operation}_saved_scan`,
      ),
    ).toHaveLength(1);
  },
);

it.each(["import", "export"] as const)(
  "keeps a %s done-then-error unconfirmed, not successful, with errors only in its receipt",
  async (operation) => {
    const native = deferred<unknown>();
    const command = `${operation}_saved_scan`;
    intercept(command, native.promise);
    const displayed = app.summary;
    const pending =
      operation === "import" ? app.importScan() : app.exportScan(original.id);
    channel(command).onmessage({ operation, phase: "done", bytes: 0 });
    native.reject("Could not synchronize the published file");
    await act(() => pending);
    expect(app.scanTransfer).toMatchObject({
      state: "failed",
      phase: "finishing",
      error: "Could not synchronize the published file",
    });
    expect(app.error).toBe("");
    expect(app.summary).toBe(displayed);
    expect(
      mock.invoke.mock.calls.filter(([name]) => name === command),
    ).toHaveLength(1);
  },
);

it("refreshes possible post-publication imports under the lock without following the library's startup selection", async () => {
  const native = deferred<unknown>(),
    list = deferred<unknown>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "import_saved_scan"
      ? native.promise
      : command === "list_saved_scans"
        ? list.promise
        : standard(command, args),
  );
  const displayed = app.summary,
    pending = app.importScan();
  native.reject("Library metadata flush failed");
  await flush();
  expect(app.scanTransfer?.state).toBe("failed");
  expect(app.libraryBusy).toBe(true);
  await app.importScan();
  await app.exportScan(original.id);
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "import_saved_scan",
    ),
  ).toHaveLength(1);
  list.resolve({ entries: [original, imported], lastOpenedId: imported.id });
  await act(() => pending);
  expect(app.saved.map((entry) => entry.id)).toEqual([
    imported.id,
    original.id,
  ]);
  expect(app.summary).toBe(displayed);
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "open_saved_scan"),
  ).toHaveLength(1);
});

it("keeps the original transfer failure if its best-effort library refresh also fails", async () => {
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "import_saved_scan"
      ? Promise.reject("Import publication could not be confirmed")
      : command === "list_saved_scans"
        ? Promise.reject("Library refresh unavailable")
        : standard(command, args),
  );
  const saved = app.saved;
  await act(() => app.importScan());
  expect(app.scanTransfer).toMatchObject({
    state: "failed",
    error: "Import publication could not be confirmed",
  });
  expect(app.saved).toBe(saved);
  expect(app.error).toBe("");
  expect(app.libraryBusy).toBe(false);
});

it.each(["success", "failure"])(
  "keeps import reconciliation independent of receipt dismissal after refresh %s",
  async (outcome) => {
    const list = deferred<unknown>();
    const standard = mock.invoke.getMockImplementation()!;
    mock.invoke.mockImplementation((command, args) =>
      command === "import_saved_scan"
        ? Promise.reject("Import publication could not be confirmed")
        : command === "list_saved_scans"
          ? list.promise
          : standard(command, args),
    );
    const displayed = app.summary,
      saved = app.saved;
    const pending = app.importScan();
    const oldProgress = channel("import_saved_scan").onmessage;
    await flush();
    expect(app.scanTransfer?.state).toBe("failed");
    expect(app.libraryBusy).toBe(true);
    app.dismissScanTransfer();
    expect(app.scanTransfer).toBeNull();
    oldProgress({ operation: "import", phase: "done", bytes: 0 });
    if (outcome === "success")
      list.resolve({
        entries: [original, imported],
        lastOpenedId: imported.id,
      });
    else list.reject("Library refresh unavailable");
    await act(() => pending);
    expect(app.saved.map((entry) => entry.id)).toEqual(
      outcome === "success" ? [imported.id, original.id] : [original.id],
    );
    if (outcome === "failure") expect(app.saved).toBe(saved);
    expect(app.scanTransfer).toBeNull();
    expect(app.libraryBusy).toBe(false);
    expect(app.summary).toBe(displayed);
    expect(app.error).toBe("");
    expect(
      mock.invoke.mock.calls.filter(
        ([command]) => command === "open_saved_scan",
      ),
    ).toHaveLength(1);
    expect(
      mock.invoke.mock.calls.filter(
        ([command]) => command === "import_saved_scan",
      ),
    ).toHaveLength(1);
  },
);

it("ignores import reconciliation results after disposal even when its receipt was dismissed", async () => {
  const list = deferred<unknown>();
  const standard = mock.invoke.getMockImplementation()!;
  mock.invoke.mockImplementation((command, args) =>
    command === "import_saved_scan"
      ? Promise.reject("Import publication could not be confirmed")
      : command === "list_saved_scans"
        ? list.promise
        : standard(command, args),
  );
  const saved = app.saved;
  const pending = app.importScan();
  await flush();
  app.dismissScanTransfer();
  await root.unmount();
  list.resolve({ entries: [original, imported], lastOpenedId: imported.id });
  await pending;
  expect(app.saved).toBe(saved);
  expect(app.scanTransfer).toBeNull();
});

it.each([
  undefined,
  false,
  {},
  { ...imported, id: "" },
  { ...imported, savedAtMs: NaN },
  { ...imported, entries: -1 },
])("does not confirm a malformed import response %j", async (value) => {
  intercept("import_saved_scan", Promise.resolve(value));
  await act(() => app.importScan());
  expect(app.scanTransfer?.state).toBe("failed");
  expect(app.scanTransfer?.entry).toBeNull();
  expect(app.saved.map((entry) => entry.id)).toEqual([original.id]);
});

it.each([undefined, null, 1, "true", {}])(
  "does not confirm a malformed export response %j",
  async (value) => {
    intercept("export_saved_scan", Promise.resolve(value));
    await act(() => app.exportScan(original.id));
    expect(app.scanTransfer?.state).toBe("failed");
  },
);

it("rejects unrelated operations, malformed counters and unknown phases", async () => {
  const native = deferred<unknown>();
  intercept("import_saved_scan", native.promise);
  const pending = app.importScan(),
    events = channel("import_saved_scan");
  const initial = app.scanTransfer;
  for (const value of [
    { operation: "export", phase: "writing", bytes: 10 },
    { operation: "import", phase: "reading", bytes: -1 },
    { operation: "import", phase: "reading", bytes: NaN },
    { operation: "import", phase: "reading", bytes: Infinity },
    { operation: "import", phase: "reading", bytes: 1.5 },
    { operation: "import", phase: "unknown", bytes: 10 },
  ])
    await act(() => events.onmessage(value as LibraryProgress));
  expect(app.scanTransfer).toBe(initial);
  expect(app.libraryProgress).toMatchObject({
    operation: "import",
    phase: "choosing",
    bytes: 0,
  });
  native.resolve(null);
  await act(() => pending);
});

it("rejects old captured progress during another library operation and after dismissal", async () => {
  const first = deferred<unknown>();
  intercept("import_saved_scan", first.promise);
  const pending = app.importScan(),
    old = channel("import_saved_scan").onmessage;
  first.resolve(imported);
  await act(() => pending);
  const report = app.scanTransfer;
  const open = deferred<ScanSummary>();
  intercept("open_saved_scan", open.promise);
  const opening = app.openSaved(original.id);
  old({ operation: "import", phase: "reading", bytes: 999 });
  expect(app.scanTransfer).toBe(report);
  expect(app.libraryProgress?.operation).toBe("open");
  open.resolve(historical);
  await act(() => opening);
  expect(app.scanTransfer).toBe(report);
  app.dismissScanTransfer();
  old({ operation: "import", phase: "done", bytes: 0 });
  expect(app.scanTransfer).toBeNull();
});

it("replaces a transfer toast only when the next toast appears and fences the previous request", async () => {
  const first = deferred<unknown>();
  intercept("import_saved_scan", first.promise);
  const pending = app.importScan(),
    old = channel("import_saved_scan").onmessage;
  first.resolve(null);
  await act(() => pending);
  expect(app.toast?.message).toContain("Import cancelled");
  const second = deferred<boolean>();
  intercept("export_saved_scan", second.promise);
  const exporting = app.exportScan(original.id);
  old({ operation: "import", phase: "done", bytes: 0 });
  expect(app.scanTransfer).toMatchObject({
    operation: "export",
    state: "pending",
    phase: "choosing",
  });
  expect(app.toast?.message).toContain("Import cancelled");
  second.resolve(true);
  await act(() => exporting);
  expect(app.toast?.message).toContain("Scan exported");
});

it("ignores callbacks and response updates after unmount", async () => {
  const native = deferred<unknown>();
  intercept("import_saved_scan", native.promise);
  const pending = app.importScan(),
    old = channel("import_saved_scan").onmessage;
  const initial = app.scanTransfer,
    saved = app.saved;
  await root.unmount();
  old({ operation: "import", phase: "reading", bytes: 100 });
  native.resolve(imported);
  await pending;
  expect(app.scanTransfer).toBe(initial);
  expect(app.saved).toBe(saved);
});

it("serializes transfer requests against each other and existing library operations", async () => {
  const native = deferred<unknown>();
  intercept("import_saved_scan", native.promise);
  const pending = app.importScan();
  await app.importScan();
  await app.exportScan(original.id);
  await app.openSaved(original.id);
  await app.compareScans(original.id, imported.id);
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "import_saved_scan",
    ),
  ).toHaveLength(1);
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "export_saved_scan",
    ),
  ).toHaveLength(0);
  expect(
    mock.invoke.mock.calls.filter(([command]) => command === "open_saved_scan"),
  ).toHaveLength(1);
  native.resolve(null);
  await act(() => pending);
  expect(app.libraryBusy).toBe(false);
});

it("retains transfer feedback through generic library operations without rerunning the transfer", async () => {
  await act(() => app.exportScan(original.id));
  const report = app.scanTransfer;
  await act(() => app.openSaved(original.id));
  app.showQueue(true);
  app.showQueue(false);
  expect(app.scanTransfer).toBe(report);
  await act(() => app.libraryOperation("save", async () => true));
  await act(() => app.compareScans(original.id, imported.id));
  expect(app.scanTransfer).toBe(report);
  expect(
    mock.invoke.mock.calls.filter(
      ([command]) => command === "export_saved_scan",
    ),
  ).toHaveLength(1);
});

it("keeps ordinary saved-scan errors in the existing banner without overwriting the transfer receipt", async () => {
  await act(() => app.exportScan(original.id));
  const report = app.scanTransfer;
  intercept("open_saved_scan", Promise.reject("Saved scan unavailable"));
  await act(() => app.openSaved(original.id));
  expect(app.error).toBe("Saved scan unavailable");
  expect(app.scanTransfer).toBe(report);
});

it("guards browser previews, busy and running scans without creating a receipt", async () => {
  mock.native = false;
  await app.importScan();
  await app.exportScan(original.id);
  mock.native = true;
  app.setBusy(true);
  await app.importScan();
  app.setBusy(false);
  app.setSummary({ ...historical, origin: "live", state: "running" });
  await app.exportScan(original.id);
  expect(app.scanTransfer).toBeNull();
  expect(
    mock.invoke.mock.calls.some(([command]) =>
      /^(import|export)_saved_scan$/.test(command),
    ),
  ).toBe(false);
});
