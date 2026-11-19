// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { flushSync } from "svelte";
import type {
  ScanChannel,
  ScanPort,
  ScanSlotsHooks,
} from "./scanSlots.svelte";
import { rootedScanSlots } from "./testScanSlots.svelte";
import type { ScanSummary, Target } from "./types";

const disk = (id: string, name: string, path: string): Target => ({
  id,
  name,
  path,
  totalBytes: 1000,
  freeBytes: 500,
  volume: true,
});
const hd = disk("hd", "Macintosh HD", "/");
const backup = disk("backup", "Backup", "/Volumes/Backup");
const usb = disk("usb", "SANDISK", "/Volumes/SANDISK");
const photos: Target = {
  id: "photos",
  name: "Photos",
  path: "/Users/ada/Photos",
  totalBytes: null,
  freeBytes: null,
};
const music: Target = {
  ...photos,
  id: "music",
  name: "Music",
  path: "/Users/ada/Music",
};
const summary = (
  scanId: string,
  state: ScanSummary["state"],
  entries = 2,
): ScanSummary => ({
  scanId,
  rootId: 0,
  state,
  workers: 1,
  workerStats: [],
  finalizationUs: 0,
  entries,
  allocatedBytes: 2048,
  elapsedMs: 1000,
  errors: 0,
});

// The fake backend: what it was asked, and the channel of every scan it started.
let channels: Map<string, ScanChannel>;
let started: string[];
let cancelled: string[];
let discarded: string[];
let summaryAsked: string[];
// Start_scan failures by target id.
let refused: Map<string, string>;
// While set, start_scan answers only once it resolves (its channel exists from the call on).
let startGate: Promise<void> | null;
// Runs while scan_summary is being fetched, to let an event beat it.
let duringSummary: (() => void) | null;
let summaryFails: boolean;
// The app's side.
let begun: { target: Target; takeView: boolean; batch: boolean }[];
let events: { scanId: string; state: string; restarted: boolean }[];
let fetched: string[];
let gatesOpen: boolean;
let overtaken: boolean;
let dealtWith: boolean;
let listed: Target[];
let scans: ReturnType<typeof rootedScanSlots>["scans"];
let dispose: () => void;

const port: ScanPort = {
  channel: () => ({ onmessage: () => {} }),
  async startScan(targetId, onEvent) {
    const error = refused.get(targetId);
    if (error) throw new Error(error);
    const id = `scan-${targetId}-${started.length + 1}`;
    started.push(id);
    channels.set(id, onEvent);
    if (startGate) await startGate;
    return id;
  },
  async cancelScan(scanId) {
    cancelled.push(scanId);
  },
  async discardScan(scanId) {
    discarded.push(scanId);
  },
  async scanSummary(scanId) {
    summaryAsked.push(scanId);
    duringSummary?.();
    if (summaryFails) throw new Error("Summary unavailable");
    return summary(scanId, "running");
  },
};
const hooks: ScanSlotsHooks = {
  begin(target, how) {
    if (!gatesOpen) return null;
    begun.push({ target, ...how });
    return {
      current: () => !overtaken,
      accepted: () => {},
      event: (s, restarted) =>
        events.push({ scanId: s.scanId, state: s.state, restarted }),
      fetched: (s) => fetched.push(s.scanId),
      failed: () => dealtWith,
    };
  },
};
const idOf = (targetId: string) =>
  started.filter((id) => id.startsWith(`scan-${targetId}-`)).at(-1)!;
const startsOf = (targetId: string) =>
  started.filter((id) => id.startsWith(`scan-${targetId}-`));
const emit = (scanId: string, state: ScanSummary["state"], entries = 2) => {
  channels.get(scanId)!.onmessage(summary(scanId, state, entries));
  flushSync();
};
async function settle() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
}
beforeEach(() => {
  channels = new Map();
  started = [];
  cancelled = [];
  discarded = [];
  summaryAsked = [];
  refused = new Map();
  startGate = null;
  duringSummary = null;
  summaryFails = false;
  begun = [];
  events = [];
  fetched = [];
  gatesOpen = true;
  overtaken = false;
  dealtWith = false;
  listed = [hd, backup, usb];
  ({ scans, dispose } = rootedScanSlots(port, hooks));
});
afterEach(() => dispose());

it("replaces only its own slot when a disk is scanned again", async () => {
  await scans.start(hd);
  await scans.start(backup);
  const other = scans.slots["/Volumes/Backup"];
  const first = idOf("hd");
  emit(first, "completed");
  await scans.start(hd);
  expect(idOf("hd")).not.toBe(first);
  expect(scans.slots["/"].scanId).toBe(idOf("hd"));
  expect(scans.slots["/Volumes/Backup"]).toBe(other);
  expect(Object.keys(scans.slots).sort()).toEqual(["/", "/Volumes/Backup"]);
});

it("lets a scan start only while its own slot is not scanning", async () => {
  await scans.start(hd);
  expect(scans.busy(hd)).toBe(true);
  expect(scans.busy(backup)).toBe(false);
  expect(scans.folderBusy()).toBe(false);
  // A busy slot is refused without asking the backend or the app.
  await scans.start(hd);
  expect(started).toHaveLength(1);
  expect(begun).toHaveLength(1);
  await scans.start(backup);
  expect(started).toHaveLength(2);
  // Every folder scan shares one slot.
  await scans.start(photos);
  expect(scans.busy(photos)).toBe(true);
  expect(scans.busy(music)).toBe(true);
  expect(scans.folderBusy()).toBe(true);
  await scans.start(music);
  expect(started).toHaveLength(3);
  emit(idOf("photos"), "completed");
  expect(scans.busy(music)).toBe(false);
  expect(scans.busy(hd)).toBe(true);
});

it("counts a scan that has not reported yet as scanning", async () => {
  summaryFails = true;
  await expect(scans.start(hd)).rejects.toThrow(
    "Summary unavailable",
  );
  expect(scans.slots["/"].summary).toBeNull();
  expect(scans.busy(hd)).toBe(true);
  expect(scans.anyRunning).toBe(true);
  emit(idOf("hd"), "running", 7);
  expect(scans.slots["/"].summary?.entries).toBe(7);
});

it("rejects with the backend's refusal unless the app deals with it", async () => {
  refused.set("hd", "Other scans are holding every file.");
  await expect(scans.start(hd)).rejects.toThrow("holding every");
  expect(scans.slots).toEqual({});
  dealtWith = true;
  await expect(scans.start(hd)).resolves.toBeUndefined();
});

it("starts nothing when the app's gates say no", async () => {
  gatesOpen = false;
  await scans.start(hd);
  expect(started).toHaveLength(0);
  expect(scans.slots).toEqual({});
});

it("counts starts that wait on the backend, and files a scan that reports before its start returns", async () => {
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const first = scans.start(hd);
  const second = scans.start(backup);
  await settle();
  expect(scans.busy(hd)).toBe(true);
  emit(idOf("hd"), "running", 4);
  expect(scans.slots["/"].scanId).toBe(idOf("hd"));
  expect(scans.slots["/"].summary?.entries).toBe(4);
  release();
  await Promise.all([first, second]);
  expect(Object.keys(scans.slots).sort()).toEqual(["/", "/Volumes/Backup"]);
});

it("takes the summary fetched after the reply only while the start has the view and no event came first", async () => {
  await scans.start(hd);
  expect(fetched).toEqual([idOf("hd")]);
  expect(scans.slots["/"].summary?.entries).toBe(2);
  // An event that beats the fetch wins.
  duringSummary = () => channels.get(idOf("backup"))!.onmessage(summary(idOf("backup"), "running", 77));
  await scans.start(backup);
  expect(scans.slots["/Volumes/Backup"].summary?.entries).toBe(77);
  expect(fetched).toEqual([idOf("hd")]);
  // A start that another one overtook never asks.
  duringSummary = null;
  overtaken = true;
  await scans.start(usb);
  expect(summaryAsked).not.toContain(idOf("usb"));
  expect(scans.slots["/Volumes/SANDISK"].summary).toBeNull();
});

it("ignores a discarded scan's late events", async () => {
  await scans.start(backup);
  const id = idOf("backup");
  expect(scans.discard(backup)).toBe(id);
  expect(discarded).toEqual([id]);
  expect(scans.slots).toEqual({});
  emit(id, "running", 50);
  emit(id, "cancelled");
  expect(scans.slots).toEqual({});
  expect(scans.busy(backup)).toBe(false);
  // Nothing to discard on a disk with no scan.
  expect(scans.discard(usb)).toBeNull();
  expect(discarded).toEqual([id]);
});

it("counts a discarded scan that is still running until its end event", async () => {
  await scans.start(backup);
  const id = idOf("backup");
  scans.discard(backup);
  // The slot is gone and does not stop another scan; the backend still counts the walk.
  expect(scans.slots).toEqual({});
  expect(scans.busy(backup)).toBe(false);
  expect(scans.anyRunning).toBe(true);
  emit(id, "running", 50);
  expect(scans.anyRunning).toBe(true);
  emit(id, "cancelled");
  expect(scans.anyRunning).toBe(false);
});

it("discards a scan whose disk was ejected before its start heard back", async () => {
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const start = scans.start(backup);
  await settle();
  const id = idOf("backup");
  // No event and no reply yet: the slot is not filed, but the eject still counts.
  expect(scans.discard(backup)).toBeNull();
  release();
  await start;
  expect(discarded).toEqual([id]);
  expect(scans.slots).toEqual({});
  expect(summaryAsked).toEqual([]);
  expect(scans.anyRunning).toBe(true);
  emit(id, "cancelled");
  expect(scans.slots).toEqual({});
  expect(scans.anyRunning).toBe(false);
});

it("discards a rescan whose disk was ejected while its old scan still had a slot", async () => {
  await scans.start(hd);
  const old = idOf("hd");
  emit(old, "completed");
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const rescan = scans.start(hd);
  await settle();
  const fresh = idOf("hd");
  expect(scans.discard(hd)).toBe(old);
  expect(discarded).toEqual([old]);
  release();
  await rescan;
  expect(discarded).toEqual([old, fresh]);
  expect(scans.slots).toEqual({});
  expect(summaryAsked).not.toContain(fresh);
});

it("discards a scan whose disk was ejected before its first event", async () => {
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const start = scans.start(backup);
  await settle();
  const id = idOf("backup");
  scans.discard(backup);
  emit(id, "running", 3);
  expect(scans.slots).toEqual({});
  expect(discarded).toEqual([id]);
  release();
  await start;
  expect(discarded).toEqual([id]);
  expect(scans.slots).toEqual({});
  expect(scans.anyRunning).toBe(true);
  emit(id, "cancelled");
  expect(scans.anyRunning).toBe(false);
});

it("has nothing to wait for when the discarded scan had ended", async () => {
  await scans.start(backup);
  emit(idOf("backup"), "completed");
  scans.discard(backup);
  expect(scans.slots).toEqual({});
  expect(scans.anyRunning).toBe(false);
});

it("changes its signature when a scan starts, ends or is dropped, not on progress", async () => {
  const empty = scans.signature;
  await scans.start(hd);
  const running = scans.signature;
  expect(running).not.toBe(empty);
  emit(idOf("hd"), "running", 40);
  expect(scans.signature).toBe(running);
  emit(idOf("hd"), "completed");
  expect(scans.signature).not.toBe(running);
  scans.drop(idOf("hd"));
  expect(scans.signature).toBe(empty);
});

it("drops a scan by its id", async () => {
  await scans.start(hd);
  const id = idOf("hd");
  emit(id, "completed");
  scans.drop("unknown");
  expect(Object.keys(scans.slots)).toEqual(["/"]);
  scans.drop(id);
  expect(scans.slots).toEqual({});
});

it("reports the batch only while two or more of its scans run, counting the ones that ended", async () => {
  expect(scans.batchProgress).toBeNull();
  await scans.start(hd);
  expect(scans.batchProgress).toBeNull();
  await scans.start(backup);
  expect(scans.batchProgress).toEqual({ done: 0, total: 2 });
  await scans.start(usb);
  expect(scans.batchProgress).toEqual({ done: 0, total: 3 });
  emit(idOf("hd"), "completed");
  expect(scans.batchProgress).toEqual({ done: 1, total: 3 });
  emit(idOf("backup"), "cancelled");
  // One scan left: the indicator goes away, and the batch ends with it.
  expect(scans.batchProgress).toBeNull();
  emit(idOf("usb"), "failed");
  expect(scans.batchProgress).toBeNull();
  // A new batch counts from zero.
  await scans.start(hd);
  await scans.start(backup);
  expect(scans.batchProgress).toEqual({ done: 0, total: 2 });
});

it("counts a scan started on its own as part of the batch", async () => {
  await scans.start(backup);
  expect(scans.batchProgress).toBeNull();
  await scans.start(usb);
  expect(scans.batchProgress).toEqual({ done: 0, total: 2 });
});

it("cancels a scanning disk, starts it again once it has ended, and starts the idle disks at once", async () => {
  await scans.start(backup);
  const old = idOf("backup");
  const done = scans.scanAll(() => listed, { takeView: () => false });
  await settle();
  expect(scans.scanningAll).toBe(true);
  expect(cancelled).toEqual([old]);
  expect(startsOf("hd")).toHaveLength(1);
  expect(startsOf("usb")).toHaveLength(1);
  // The cancelled scan has not ended, so it is not started again yet.
  expect(startsOf("backup")).toHaveLength(1);
  emit(old, "cancelled");
  expect(await done).toEqual([]);
  expect(startsOf("backup")).toHaveLength(2);
  expect(scans.slots["/Volumes/Backup"].scanId).toBe(idOf("backup"));
  expect(scans.scanningAll).toBe(false);
  // The old scan did not end on its own account.
  expect(events.find((e) => e.scanId === old && e.state === "cancelled")).toEqual({
    scanId: old,
    state: "cancelled",
    restarted: true,
  });
});

it("does not call a scan's end a restart when nothing restarts it", async () => {
  await scans.start(backup);
  emit(idOf("backup"), "cancelled");
  expect(events.at(-1)).toEqual({
    scanId: idOf("backup"),
    state: "cancelled",
    restarted: false,
  });
});

it("tells the app which starts belong to Scan all and may take the view", async () => {
  await scans.start(photos);
  await scans.scanAll(() => [hd, backup], {
    takeView: (key) => key === "/Volumes/Backup",
  });
  expect(
    begun.map(({ target, takeView, batch }) => [target.id, takeView, batch]),
  ).toEqual([
    ["photos", true, false],
    ["hd", false, true],
    ["backup", true, true],
  ]);
});

it("keeps starting the other disks when some are refused, and reports every refusal together", async () => {
  refused.set("hd", "No room for another scan.");
  refused.set("usb", "Not allowed.");
  const failures = await scans.scanAll(() => listed, { takeView: () => false });
  expect(failures.sort()).toEqual([
    "Macintosh HD: Error: No room for another scan.",
    "SANDISK: Error: Not allowed.",
  ]);
  expect(startsOf("backup")).toHaveLength(1);
  expect(Object.keys(scans.slots)).toEqual(["/Volumes/Backup"]);
  expect(scans.scanningAll).toBe(false);
});

it("leaves a failure to the app when it deals with it", async () => {
  dealtWith = true;
  refused.set("hd", "FULL_DISK_ACCESS_OFF");
  expect(await scans.scanAll(() => [hd], { takeView: () => false })).toEqual(
    [],
  );
});

it("keeps counting a disk as running while its cancelled scan waits to start again", async () => {
  await scans.start(hd);
  await scans.start(backup);
  let release!: () => void;
  startGate = new Promise<void>((resolve) => (release = resolve));
  const done = scans.scanAll(() => [hd, backup], { takeView: () => false });
  await settle();
  emit(idOf("hd"), "cancelled");
  emit(idOf("backup"), "cancelled");
  await settle();
  // Every old scan has ended and no new one is accepted yet.
  expect(startsOf("hd")).toHaveLength(2);
  expect(scans.batchProgress).toEqual({ done: 0, total: 2 });
  release();
  await done;
  expect(scans.batchProgress).toEqual({ done: 0, total: 2 });
});

it("skips a disk that was ejected while its scan was being cancelled", async () => {
  await scans.start(backup);
  await scans.start(usb);
  const done = scans.scanAll(() => listed, { takeView: () => false });
  await settle();
  listed = [hd, backup];
  emit(idOf("backup"), "cancelled");
  emit(idOf("usb"), "cancelled");
  expect(await done).toEqual([]);
  expect(startsOf("usb")).toHaveLength(1);
  expect(startsOf("backup")).toHaveLength(2);
  expect(startsOf("hd")).toHaveLength(1);
});

it("gives up on a scan that does not stop within 30 seconds and reports it", async () => {
  await scans.start(backup);
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    const done = scans.scanAll(() => [hd, backup], { takeView: () => false });
    await vi.advanceTimersByTimeAsync(29_000);
    expect(scans.scanningAll).toBe(true);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(await done).toEqual(["Backup: The scan did not stop in time."]);
    expect(startsOf("backup")).toHaveLength(1);
    expect(startsOf("hd")).toHaveLength(1);
    expect(scans.scanningAll).toBe(false);
  } finally {
    vi.useRealTimers();
  }
});
