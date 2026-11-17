import { Channel, invoke } from "@tauri-apps/api/core";
import type { ScanSummary, Target } from "./types";

/** The slot every folder scan shares; a disk's slot is keyed by its path. */
export const FOLDER_SLOT = "folder";

/** One live scan, mirroring the backend slot of the same key. */
export type ScanSlot = {
  target: Target;
  scanId: string;
  /** Null until the first progress event of a scan that has just started. */
  summary: ScanSummary | null;
};
export type ScanSlots = Readonly<Record<string, ScanSlot>>;

/** The one place-to-slot rule, the backend's (`start_scan_impl`): a volume target is its disk's slot, anything else is the folder slot. */
export const slotKey = (target: Target) =>
  target.volume ? target.path : FOLDER_SLOT;

export const slotScanning = (slot: ScanSlot) =>
  !slot.summary || slot.summary.state === "running";

/** One scan's event channel: the backend sends its summaries to `onmessage`. */
export type ScanChannel = { onmessage: (summary: ScanSummary) => void };

/** What the slots ask of the backend. Production wraps Tauri; tests pass a fake. */
export type ScanPort = {
  channel(): ScanChannel;
  startScan(targetId: string, onEvent: ScanChannel): Promise<string>;
  rescanSavedScan(
    scanId: string,
    onEvent: ScanChannel,
  ): Promise<{ scanId: string; target: Target }>;
  cancelScan(scanId: string): Promise<unknown>;
  discardScan(scanId: string): Promise<unknown>;
  scanSummary(scanId: string): Promise<ScanSummary>;
};

export const tauriScanPort: ScanPort = {
  channel: () => new Channel<ScanSummary>(),
  startScan: (targetId, onEvent) =>
    invoke<string>("start_scan", { targetId, onEvent }),
  rescanSavedScan: (scanId, onEvent) =>
    invoke("rescan_saved_scan", { scanId, onEvent }),
  cancelScan: (scanId) => invoke("cancel_scan", { scanId }),
  discardScan: (scanId) => invoke("discard_scan", { scanId }),
  scanSummary: (scanId) => invoke<ScanSummary>("scan_summary", { scanId }),
};

/** A new scan of a target, or of the location a saved scan measured. */
export type StartRequest = { target: Target } | { savedScanId: string };

/**
 * What the app does for one start: the view's side of it. The slots call these in the order the
 * start unfolds and never look at the view themselves.
 */
export type StartView = {
  /** Settles before the backend is asked; a rejection fails the start. */
  ready?: Promise<unknown>;
  /** True while this start may still take the view: no newer start or navigation overtook it. */
  current(): boolean;
  /** The scan has an id (its first event or the reply, once). Its slot is already filed. */
  accepted(scanId: string): void;
  /** An event of this start's scan, after its slot recorded it. `restarted`: Scan all cancelled it to start again. */
  event(summary: ScanSummary, restarted: boolean): void;
  /** A saved scan's rescan was registered as this target, after its slot was filed. */
  target(target: Target): void;
  /** The scan's summary fetched after its id arrived, when no event beat it. */
  fetched(summary: ScanSummary): void;
  /** The backend refused the start. True when the app dealt with it, else the start rejects. */
  failed(error: unknown): boolean;
};

export type ScanSlotsHooks = {
  /**
   * A start is about to go ahead: the app's own gates, then the view. Null when a gate says no.
   * `takeView` is false for the starts of Scan all that only fill their sidebar row.
   */
  begin(
    request: StartRequest,
    how: { takeView: boolean; batch: boolean },
  ): StartView | null;
};

// Cancelled scans get this long to report their end before Scan all gives up on the disk.
const STOP_WAIT_MS = 30_000;

/**
 * The live scans, one per slot: a disk's path, or FOLDER_SLOT for every folder scan. The backend
 * keeps the same slots. A scan stays until the next scan of its slot or its disk's ejection.
 * Must be created while a component initialises (it owns an effect).
 */
export function createScanSlots(port: ScanPort, hooks: ScanSlotsHooks) {
  let slots = $state.raw<ScanSlots>({});
  function put(key: string, slot: ScanSlot) {
    slots = { ...slots, [key]: slot };
  }
  function drop(scanId: string) {
    slots = Object.fromEntries(
      Object.entries(slots).filter(([, slot]) => slot.scanId !== scanId),
    );
  }
  // Keeps a slot's summary current. A scan that was replaced or discarded no longer has a slot.
  function patch(scanId: string, change: Partial<ScanSummary>) {
    const key = Object.keys(slots).find((k) => slots[k].scanId === scanId);
    const slot = key ? slots[key] : undefined;
    if (key && slot?.summary)
      put(key, { ...slot, summary: { ...slot.summary, ...change } });
  }
  function record(s: ScanSummary) {
    const key = Object.keys(slots).find((k) => slots[k].scanId === s.scanId);
    if (key) put(key, { ...slots[key], summary: s });
  }
  // Entries are matched against the scan of their own disk, so any slot's scan completing, ending,
  // being replaced, cleaned up or dropped can change a status, not only the displayed scan.
  const signature = $derived(
    Object.values(slots)
      .map((slot) => `${slot.scanId}:${slot.summary?.state ?? ""}`)
      .sort()
      .join("|"),
  );
  // Discarded scans that were still running. The backend counts them until they report an end, so
  // nothing else can start before then, whichever disk it is on.
  let draining = $state.raw<readonly string[]>([]);
  const anyRunning = $derived(
    draining.length > 0 || Object.values(slots).some(slotScanning),
  );
  // Starts that have not heard back from the backend yet. Several can overlap, one per slot.
  let starts = $state.raw(0);
  // Scans run side by side, one per slot: a scan can only start if its own slot is not scanning.
  function slotBusy(key: string) {
    const slot = slots[key];
    return !!slot && slotScanning(slot);
  }
  // Slots scanned since every scan last ended: the batch the Disks header reports on. A restarted slot
  // keeps its entry, so a restart counts once. It resets when nothing runs or starts.
  // True from the press until every disk has been handed to start, restarts' waits included.
  let scanningAll = $state.raw(false);
  let batchKeys = $state.raw<readonly string[]>([]);
  // Slots whose scan Scan all cancelled and has not started again yet: still running for the indicator.
  let awaitingRestart = $state.raw<readonly string[]>([]);
  function trackBatch(key: string) {
    if (!batchKeys.includes(key)) batchKeys = [...batchKeys, key];
  }
  $effect(() => {
    if (!starts && !scanningAll && !Object.values(slots).some(slotScanning))
      batchKeys = [];
  });
  // Only while two or more scans run: how many of the batch's slots have ended (finished, cancelled or failed).
  const batchProgress = $derived.by(() => {
    const keys = batchKeys.filter((key) => slots[key]);
    const running = keys.filter(
      (key) => slotScanning(slots[key]) || awaitingRestart.includes(key),
    ).length;
    return running >= 2
      ? { done: keys.length - running, total: keys.length }
      : null;
  });
  // Scans that Scan all disks cancelled to start again, with what resolves when they report an end.
  const restarting = new Map<string, () => void>();
  // Disk starts that have no id yet, by slot key. An eject then can only mark the start, which
  // discards its scan once the id arrives instead of filing a slot for a disk that is gone.
  const unfiled = new Map<string, { ejected: boolean }>();

  async function run(
    request: StartRequest,
    how: { takeView: boolean; batch: boolean },
  ) {
    const place = "target" in request ? request.target : null;
    // rescan_saved_scan registers a folder target, so a saved scan is rescanned in the folder slot.
    if (slotBusy(place ? slotKey(place) : FOLDER_SLOT)) return;
    const view = hooks.begin(request, how);
    if (!view) return;
    ++starts;
    const pending = { ejected: false };
    if (place) unfiled.set(slotKey(place), pending);
    let startedId: string | null = null;
    // Events that arrive before a saved scan's rescan target is known.
    let latest: ScanSummary | null = null;
    const accept = (id: string) => {
      if (startedId !== null) return;
      startedId = id;
      if (place && unfiled.get(slotKey(place)) === pending)
        unfiled.delete(slotKey(place));
      if (pending.ejected) {
        void port.discardScan(id).catch(() => {});
        draining = [...draining, id];
        return;
      }
      // The backend replaced this slot's scan at once; other slots keep theirs. The slot is tracked
      // whatever the user opened since, so a start that another one overtook is not lost.
      if (place) {
        put(slotKey(place), { target: place, scanId: id, summary: null });
        trackBatch(slotKey(place));
      }
      view.accepted(id);
    };
    let updates = 0;
    const channel = port.channel();
    // A channel carries one scan's events, so everything on it belongs to that scan's slot.
    channel.onmessage = (s) => {
      accept(s.scanId);
      ++updates;
      latest = s;
      if (s.state !== "running")
        draining = draining.filter((id) => id !== s.scanId);
      record(s);
      if (s.state !== "running") restarting.get(s.scanId)?.();
      view.event(s, restarting.has(s.scanId));
    };
    try {
      if (view.ready) await view.ready;
      let id: string;
      if ("savedScanId" in request) {
        const result = await port.rescanSavedScan(
          request.savedScanId,
          channel,
        );
        id = result.scanId;
        accept(id);
        put(slotKey(result.target), {
          target: result.target,
          scanId: id,
          summary: latest,
        });
        trackBatch(slotKey(result.target));
        view.target(result.target);
      } else {
        id = await port.startScan(request.target.id, channel);
      }
      accept(id);
      if (!pending.ejected && view.current()) {
        const beforeSummary = updates;
        const s = await port.scanSummary(id);
        if (view.current() && updates === beforeSummary) {
          record(s);
          view.fetched(s);
        }
      }
    } catch (e) {
      if (!view.failed(e)) throw e;
    } finally {
      --starts;
      if (place && unfiled.get(slotKey(place)) === pending)
        unfiled.delete(slotKey(place));
    }
  }

  // Cancels a running scan and waits for its final event, so the slot is free for a fresh start.
  async function cancelAndWait(slot: ScanSlot) {
    const ended = new Promise<void>((resolve) =>
      restarting.set(slot.scanId, resolve),
    );
    // A scan finishing its rescan cache refuses to cancel; it ends on its own soon.
    void port.cancelScan(slot.scanId).catch(() => {});
    let timer: ReturnType<typeof setTimeout> | undefined;
    await Promise.race([
      ended,
      new Promise<void>(
        (resolve) => (timer = setTimeout(resolve, STOP_WAIT_MS)),
      ),
    ]);
    clearTimeout(timer);
    restarting.delete(slot.scanId);
  }

  return {
    get slots() {
      return slots;
    },
    get signature() {
      return signature;
    },
    /** A scan, or a discarded one still ending, that nothing outside a slot may overlap. */
    get anyRunning() {
      return anyRunning;
    },
    /** A start has not heard back from the backend yet. */
    get starting() {
      return starts > 0;
    },
    get scanningAll() {
      return scanningAll;
    },
    get batchProgress() {
      return batchProgress;
    },
    /** Whether a scan of this place would meet its own slot scanning. */
    busy: (place: Target) => slotBusy(slotKey(place)),
    /** The folder slot's scan runs: where a saved scan's rescan goes. */
    folderBusy: () => slotBusy(FOLDER_SLOT),
    /** Starts a scan; a busy slot is a no-op. Rejects with the backend's refusal when the app does not deal with it. */
    start: (request: StartRequest) =>
      run(request, { takeView: true, batch: false }),
    /** Forgets a scan: replaced by a cleanup, or its disk is gone. */
    drop,
    /** Changes a scan's recorded summary. */
    patch,
    /** Ejects a disk's scan: the backend frees it, the slot is dropped. Returns its id, or null with no scan. */
    discard(place: Target) {
      const slot = slots[slotKey(place)];
      if (!slot) {
        const pending = unfiled.get(slotKey(place));
        if (pending) pending.ejected = true;
        return null;
      }
      void port.discardScan(slot.scanId).catch(() => {});
      if (slotScanning(slot)) draining = [...draining, slot.scanId];
      drop(slot.scanId);
      return slot.scanId;
    },
    /**
     * Starts a fresh scan of every disk `disks()` lists; one that is scanning is cancelled first. A
     * refused start does not stop the others. Returns the failures, one line per disk. `takeView`
     * says per slot key whether that disk's start may take the view.
     */
    async scanAll(
      disks: () => readonly Target[],
      options: { takeView: (key: string) => boolean },
    ) {
      scanningAll = true;
      const failures: string[] = [];
      try {
        await Promise.all(
          disks().map(async (disk) => {
            const key = slotKey(disk);
            const report = (e: unknown) =>
              failures.push(`${disk.name}: ${String(e)}`);
            try {
              const slot = slots[key];
              if (slot && slotScanning(slot)) {
                awaitingRestart = [...awaitingRestart, key];
                await cancelAndWait(slot);
              }
              // Ejected while waiting: nothing to scan.
              if (!disks().some((item) => item.id === disk.id)) return;
              if (slotBusy(key)) return report("The scan did not stop in time.");
              await run(
                { target: disk },
                { takeView: options.takeView(key), batch: true },
              );
            } catch (e) {
              report(e);
            } finally {
              awaitingRestart = awaitingRestart.filter((item) => item !== key);
            }
          }),
        );
      } finally {
        scanningAll = false;
      }
      return failures;
    },
  };
}
