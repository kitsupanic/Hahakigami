import { onMount, untrack } from "svelte";
import createQuickLook from "./createQuickLook.svelte";
import { createCleanupQueue, tauriQueuePort } from "./cleanupQueue.svelte";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./quickLook.css";
import {
  isSortOrder,
  type SortOrder,
  type ExplorerPreferences,
} from "./sortTypes";
import { emptyFilter, filterActive, type ExplorerFilter } from "./filterTypes";
import {
  createScanSlots,
  slotKey,
  tauriScanPort,
  type StartView,
} from "./scanSlots.svelte";
import { latestOnly } from "./latestQuery";
import { countLabel } from "./uiFormat";
import type { CleanupReport, QueueView } from "./cleanupTypes";
import { cleanupResult } from "./cleanupResults";
import type { AppToast } from "./toastTypes";
import { createPerformanceHarness } from "./performanceHarness.svelte";
import {
  type FullDiskAccess,
  type NodeView,
  type Page,
  type ScanSummary,
  type Target,
  PAGE_SIZE,
} from "./types";
// Mirrors the backend limit in cleanup.rs ("Select between 1 and 1,000 items.").
const MAX_TRASH_ITEMS = 1000;
const performanceBuild = import.meta.env.VITE_PERFORMANCE_BENCHMARK === "1";
export default function createApp() {
  let mapRequestEpoch = 0;
  let targets = $state.raw<Target[]>([]);
  let target = $state.raw<Target | null>(null);
  let summary = $state.raw<ScanSummary | null>(null);
  let focus = $state.raw<NodeView | null>(null);
  let crumbs = $state.raw<NodeView[]>([]);
  let page = $state.raw<Page>({ items: [], total: 0 });
  // Nodes read (as list rows, branch rows, map nodes or a refresh) while their scan was finished, so they cannot
  // change: a selection refresh skips them.
  const settledNodes = new WeakSet<NodeView>();
  const markSettled = (nodes: readonly NodeView[]) =>
    nodes.forEach((node) => settledNodes.add(node));
  let map = $state.raw<NodeView[]>([]);
  let mapColorMode = $state<"folder" | "fileType">("folder");
  const queryFileTypeMap = latestOnly();
  const queryMap = latestOnly();
  const queryRefresh = latestOnly();
  const queryList = latestOnly();
  function changeMapColorMode(value: "folder" | "fileType") {
    mapColorMode = value;
  }
  let selection = $state.raw<NodeView[]>([]);
  let mode = $state.raw<"treemap" | "sunburst">("sunburst");
  let query = $state.raw("");
  let offset = $state.raw(0);
  // A failure with no dedicated place shows in a modal: plain words first, the raw error as a detail.
  let error = $state.raw("");
  let errorContext = $state.raw<{ title: string; reason: string } | null>(null);
  function setError(
    value: string,
    context: { title: string; reason: string } | null = null,
  ) {
    error = value;
    errorContext = value ? context : null;
  }
  let targetsLoading = $state.raw(isTauri());
  let choosingFolder = $state(false);
  let folderRequest = 0;
  let disposed = false;
  // The live scans. beginStart is the app's side of every start.
  const scans = createScanSlots(tauriScanPort, { begin: beginStart });
  // Set by what holds the session (trash review, cleanup). A start in flight blocks only its own
  // slot (`scans.busy`), so a stalled disk never freezes the rest of the UI.
  let busy = $state.raw(false);
  let cancelling = $state.raw(false);
  let trash = $state.raw<{
    // The scan the review was made for; commit must target it, not whatever is on screen later.
    scanId: string;
    queued?: boolean;
    token: string;
    items: NodeView[];
  } | null>(null);
  let queueOpen = $state(false);
  // Home shows the welcome page without discarding the loaded or running scan.
  let homeOpen = $state(false);
  let cleanupReport = $state.raw<CleanupReport | null>(null);
  let cleanupReportId = 0;
  function dismissCleanupReport() {
    cleanupReport = null;
  }
  function updateReportQueue(
    id: number | undefined,
    state: CleanupReport["queueState"],
    view?: QueueView,
    error: string | null = null,
  ) {
    if (!cleanupReport || cleanupReport.id !== id) return;
    cleanupReport = {
      ...cleanupReport,
      queueState: state,
      remainingQueued:
        state === "confirmed" && view ? view.entries.length : null,
      queueWarning: view?.warning ?? null,
      queueError: error,
    };
  }
  let clearQueueOpen = $state(false);
  const cleanupQueue = createCleanupQueue(tauriQueuePort, {
    available: () => !performanceBuild && isTauri(),
    mutable: () => !choosingFolder && !busy,
    reporter() {
      const id = cleanupReport?.id;
      return (state, view, error = null) =>
        updateReportQueue(id, state, view, error);
    },
    mutated() {
      if (trash?.queued) trash = null;
      clearQueueOpen = false;
    },
    selection: () => selection,
    shownScanId: () => shown?.summary.scanId ?? null,
    diskName: (root) => targets.find((item) => item.path === root)?.name,
    addSelection: () => addQueue(),
  });
  const queue = $derived(cleanupQueue.view);
  const queueBusy = $derived(cleanupQueue.busy);
  const refreshQueue = cleanupQueue.refresh;
  function showQueue(value: boolean) {
    if (value && (busy || trash || queueBusy || choosingFolder)) return;
    quickLook.dismiss();
    queueOpen = value;
    if (value) {
      homeOpen = false;
      void refreshQueue();
    } else if (summary && !displayedSlot) {
      // A rescan replaced these hidden results while Home or the queue was open: their scan is gone.
      const key = liveTarget && slotKey(liveTarget);
      if (key && scans.slots[key]) showSlot(key);
      else resetView(null, null);
    }
  }
  async function addQueue() {
    if (
      !summary ||
      summary.state !== "completed" ||
      sessionBlocked ||
      selection.some((n) => n.excluded) ||
      !selection.length
    )
      return;
    quickLook.dismiss();
    await cleanupQueue.add(
      summary.scanId,
      selection.map((n) => n.id),
    );
  }
  // A disk of the Disks list, by its path.
  const listedDisk = (path: string) =>
    targets.find((t) => t.volume && t.path === path);
  // The backend gave the listed `disk` a new id and dropped the old one: its row takes `next`, under its listed name.
  function replaceDisk(disk: Target, next: Target) {
    if (disk.id === next.id) return disk;
    const renamed = { ...next, name: disk.name };
    targets = targets.map((t) => (t === disk ? renamed : t));
    return renamed;
  }
  // Puts a place into the targets list and returns what to select. A listed disk is not added twice:
  // its rescan uses the same slot. keepListed leaves a folder that is already listed where it is.
  function listPlace(place: Target, keepListed = false) {
    const disk = place.volume ? listedDisk(place.path) : undefined;
    if (disk) return replaceDisk(disk, place);
    if (!keepListed || !targets.some((t) => t.id === place.id))
      targets = [
        // The backend gives a recreated folder a new id and drops the old one.
        ...targets.filter(
          (t) => t.id !== place.id && (t.volume || t.path !== place.path),
        ),
        place,
      ];
    return place;
  }
  function scanQueue(entryId: string) {
    if (sessionBlocked) return;
    // The lookup can take seconds; what the user opened meanwhile keeps the view.
    const seen = { g: generation.current, id: target?.id, trash, queueOpen };
    return cleanupQueue.scan(entryId, async (location) => {
      if (
        seen.g !== generation.current ||
        seen.id !== target?.id ||
        seen.trash !== trash ||
        seen.queueOpen !== queueOpen
      )
        return;
      target = listPlace(location);
      queueOpen = false;
      // Entries already ready in a completed scan of this slot: open it, a rescan would drop them to "Needs scan".
      const key = slotKey(location);
      const done = scans.slots[key];
      const group = cleanupQueue.groups.find((g) =>
        g.entries.some((entry) => entry.id === entryId),
      );
      if (
        done?.summary?.state === "completed" &&
        group?.entries.every(
          (entry) =>
            entry.status === "ready" && entry.matchedScanId === done.scanId,
        )
      )
        return showSlot(key);
      // Not awaited: a sleeping disk must not hold the queue busy.
      void start(location);
    });
  }
  async function prepareQueue() {
    if (
      !summary ||
      summary.state !== "completed" ||
      sessionBlocked ||
      queueBusy ||
      queue.blocked
    )
      return;
    quickLook.dismiss();
    busy = true;
    const scanId = summary.scanId;
    try {
      const review = await cleanupQueue.prepare(scanId);
      // The user may have left the queue page while the review loaded.
      if (queueOpen && summary?.scanId === scanId)
        trash = { ...review, scanId, queued: true };
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
  const quickLook = createQuickLook();
  let filters = $state.raw<ExplorerFilter>({ ...emptyFilter });
  let debouncedQuery = $state.raw("");
  let listLoading = $state.raw(false);
  const listRequest = { current: 0 };
  const filtered = $derived(filterActive(filters));
  const filterKey = $derived(JSON.stringify(filters));
  type ResultsReturn = {
    scanId: string;
    focus: NodeView;
    crumbs: NodeView[];
    query: string;
    filters: ExplorerFilter;
    sort: SortOrder;
    offset: number;
    selection: NodeView[];
    total: number;
    scroll: number;
  };
  let resultsReturn = $state.raw<ResultsReturn | null>(null);
  type NavigationView = ResultsReturn & { resultsReturn: ResultsReturn | null };
  let backHistory = $state.raw<NavigationView[]>([]);
  let forwardHistory = $state.raw<NavigationView[]>([]);
  function clearNavigationHistory() {
    backHistory = [];
    forwardHistory = [];
  }
  let listScroll = 0;
  let listScrollRestore = $state.raw<{ id: number; top: number } | null>(null);
  let scrollRestoreId = 0;
  function setListScroll(top: number) {
    if (Number.isFinite(top) && top >= 0) listScroll = top;
  }
  function finishListScrollRestore(id: number) {
    if (listScrollRestore?.id === id) listScrollRestore = null;
  }
  function clearResultsReturn() {
    resultsReturn = null;
    listScrollRestore = null;
    listScroll = 0;
  }
  function invalidateList() {
    listScrollRestore = null;
    listScroll = 0;
    quickLook.dismiss();
    ++listRequest.current;
    page = { items: [], total: 0 };
    listLoading = true;
  }
  function changeFilters(value: ExplorerFilter) {
    if (JSON.stringify(value) === filterKey) return;
    clearResultsReturn();
    invalidateList();
    filters = value;
    offset = 0;
    selection = [];
  }
  function changeQuery(value: string) {
    if (value === query) return;
    clearResultsReturn();
    invalidateList();
    query = value;
    offset = 0;
    selection = [];
  }
  $effect(() => {
    [query];
    return untrack(() => {
      const timer = setTimeout(() => (debouncedQuery = query), 250);
      return () => clearTimeout(timer);
    });
  });
  let sort = $state.raw<SortOrder>("sizeDescending");
  let sortReady = $state.raw(false);
  let sortSaving = $state.raw(false);
  let sortWarning = $state.raw<string | null>(null);
  const sortLock = { current: false };
  const sortEpoch = { current: 0 };
  const sortChoice = { current: 0 };
  onMount(() => {
    const epoch = ++sortEpoch.current,
      choice = sortChoice.current;
    let cancelled = false;
    const live = () => !cancelled && sortEpoch.current === epoch;
    if (!isTauri()) {
      sortReady = true;
      return;
    }
    // A sort choice made meanwhile (a restored view) keeps its order, but the list must still unlock.
    const current = () => sortChoice.current === choice;
    invoke<ExplorerPreferences>("get_explorer_preferences")
      .then((preferences) => {
        if (!live()) return;
        if (current()) {
          if (!preferences || !isSortOrder(preferences.sort)) {
            sortWarning =
              "Could not load the saved sort order. Using largest first.";
          } else {
            sort = preferences.sort;
            sortWarning = preferences.warning ?? null;
          }
        }
        sortReady = true;
      })
      .catch((failure) => {
        if (!live()) return;
        if (current())
          sortWarning = `Could not load the saved sort order. Using largest first. ${String(failure)}`;
        sortReady = true;
      });
    return () => {
      cancelled = true;
      ++sortEpoch.current;
    };
  });
  async function persistSort(order: SortOrder) {
    if (sortLock.current || !sortReady) return;
    sortLock.current = true;
    sortSaving = true;
    const epoch = sortEpoch.current;
    try {
      const preferences = await invoke<ExplorerPreferences>(
        "set_explorer_sort",
        { sort: order },
      );
      if (!preferences || preferences.sort !== order)
        throw new Error("Invalid preference response");
      if (epoch === sortEpoch.current && sort === order)
        sortWarning = preferences.warning ?? null;
    } catch (failure) {
      if (epoch === sortEpoch.current && sort === order)
        sortWarning = `Sort order is on now, but it couldn't be saved. ${String(failure)}`;
    } finally {
      sortLock.current = false;
      if (epoch === sortEpoch.current) {
        sortSaving = false;
        // Returning can restore an earlier choice while the browse choice saves.
        // Serialize that final choice after the outstanding write completes.
        if (sort !== order) void persistSort(sort);
      }
    }
  }
  function changeSort(order: SortOrder) {
    if (!sortReady || sortLock.current || !isSortOrder(order) || order === sort)
      return;
    ++sortChoice.current;
    invalidateList();
    sort = order;
    offset = 0;
    selection = [];
    void persistSort(order);
  }
  let access = $state.raw<FullDiskAccess | null>(null);
  let accessOpen = $state.raw(false);
  let accessBusy = $state.raw(false);
  let accessError = $state.raw("");
  // The user came back from System Settings and Full Disk Access is still off.
  let accessReturned = $state.raw(false);
  // The dialog was opened because a scan is waiting on Full Disk Access.
  let accessPrompt = $state.raw(false);
  let openedSettings = false;
  let blockedScan: {
    target: Target | null;
    // The blocked request was Scan all disks: "Scan anyway" runs it again for every disk.
    scanAll?: true;
  } | null = null;
  const generation = { current: 0 };
  const navigation = $state({ current: 0 });
  // The Scan target of the scan on screen.
  let liveTarget = $state.raw<Target | null>(null);
  const activeId: { current: string | null } = { current: null };
  const blocked = $derived(busy || choosingFolder);
  const running = $derived(summary?.state === "running");
  const sessionBlocked = $derived(blocked || quickLook.pending || queueBusy);
  const previewDialog = $derived(accessOpen || !!trash || clearQueueOpen);
  $effect(() => {
    [accessOpen, trash, clearQueueOpen];
    return untrack(() => {
      if (previewDialog) quickLook.dismiss();
    });
  });
  function previewEligible(node: NodeView) {
    return (
      isTauri() &&
      !!summary &&
      summary.state === "completed" &&
      node.kind === "file" &&
      !node.excluded &&
      !blocked &&
      !previewDialog &&
      !quickLook.pending
    );
  }
  function previewRow(node: NodeView) {
    if (selection.length > 1 || !previewEligible(node)) return false;
    if (
      selection.length === 1 &&
      quickLook.status.open &&
      quickLook.status.scanId === summary!.scanId &&
      quickLook.status.nodeId === node.id &&
      selection[0].id === node.id
    ) {
      quickLook.dismiss();
      return true;
    }
    quickLook.dismiss();
    selection = [node];
    return quickLook.open({ scanId: summary!.scanId, nodeId: node.id });
  }
  const previewNode = $derived(selection.length === 1 ? selection[0] : null);
  const previewVisible = $derived(
    quickLook.status.open &&
      quickLook.status.scanId === summary?.scanId &&
      quickLook.status.nodeId === previewNode?.id,
  );
  // The slot whose live results are on screen.
  const displayedSlot = $derived(
    summary
      ? (Object.keys(scans.slots).find(
          (k) => scans.slots[k].scanId === summary!.scanId,
        ) ?? null)
      : null,
  );
  // The scan on screen, its slot and Scan target; null on Home or with none.
  const shown = $derived(
    summary && !homeOpen
      ? { summary, slot: displayedSlot, target: liveTarget }
      : null,
  );
  // A null summary is a scan that has not reported yet: an empty view that its first event fills.
  function resetView(s: ScanSummary | null, scanId = s?.scanId ?? null) {
    homeOpen = false;
    clearResultsReturn();
    clearNavigationHistory();
    quickLook.dismiss();
    ++generation.current;
    ++navigation.current;
    activeId.current = scanId;
    summary = s;
    focus = null;
    crumbs = [];
    selection = [];
    map = [];
    page = { items: [], total: 0 };
    offset = 0;
    query = "";
    debouncedQuery = "";
    filters = { ...emptyFilter };
    ++listRequest.current;
    trash = null;
    cancelling = false;
  }
  let toast = $state.raw<AppToast | null>(null);
  let toastId = 0;
  function showToast(message: string) {
    toast = { id: ++toastId, message };
  }
  function dismissToast() {
    toast = null;
  }
  async function loadAccess(current = () => true) {
    accessBusy = true;
    accessError = "";
    try {
      const info = await invoke<FullDiskAccess>("full_disk_access_status");
      if (current()) access = info;
      return info;
    } catch (e) {
      if (current()) accessError = String(e);
    } finally {
      if (current()) accessBusy = false;
    }
  }
  function openAccess() {
    quickLook.dismiss();
    accessError = "";
    accessOpen = true;
    void loadAccess();
  }
  function closeAccess() {
    blockedScan = null;
    accessPrompt = false;
    openedSettings = false;
    accessReturned = false;
    accessError = "";
    accessOpen = false;
  }
  // Only starting a scan waits on Full Disk Access; unknown never blocks.
  function blockScan(request: NonNullable<typeof blockedScan>) {
    blockedScan = request;
    accessPrompt = true;
    openedSettings = false;
    accessReturned = false;
    accessOpen = true;
  }
  function scanBlocked(request: NonNullable<typeof blockedScan>) {
    if (access?.status !== "off" || access.scanAnyway) return false;
    blockScan(request);
    return true;
  }
  async function scanAnyway() {
    const request = blockedScan;
    accessBusy = true;
    accessError = "";
    try {
      access = await invoke<FullDiskAccess>("scan_without_full_disk_access");
    } catch (e) {
      accessError = String(e);
      return;
    } finally {
      accessBusy = false;
    }
    closeAccess();
    if (request?.scanAll) await scanAllDisks();
    else if (request) await start(request.target);
  }
  async function openSettings() {
    try {
      await invoke("open_full_disk_access");
      openedSettings = true;
    } catch (e) {
      accessError = String(e);
    }
  }
  async function recheckAccess() {
    const info = await loadAccess();
    if (!info || !accessOpen) return;
    if (info.status === "off") accessReturned = true;
    else {
      blockedScan = null;
      openedSettings = false;
      accessReturned = false;
    }
  }
  async function relaunch() {
    if (quickLook.isPending()) return;
    quickLook.dismiss();
    accessBusy = true;
    accessError = "";
    try {
      await invoke("restart_app");
    } catch (e) {
      accessError = String(e);
    } finally {
      accessBusy = false;
    }
  }
  async function chooseFolder(): Promise<Target | null | void> {
    if (
      !isTauri() ||
      disposed ||
      targetsLoading ||
      sessionBlocked ||
      quickLook.isPending() ||
      accessOpen ||
      !!trash ||
      clearQueueOpen
    )
      return;
    choosingFolder = true;
    const request = ++folderRequest;
    // Another scan may take the view while the dialog is open; the chosen folder still counts.
    const current = () => !disposed && request === folderRequest;
    quickLook.dismiss();
    try {
      const chosen = await invoke<Target | null>("choose_folder");
      if (!current()) return;
      if (chosen) target = listPlace(chosen, true);
      return chosen;
    } catch (e) {
      if (current())
        fail(e, {
          title: "Couldn’t choose the folder",
          reason:
            "Hahakigami could not open the folder picker. Try again, or pick a location from the sidebar.",
        });
    } finally {
      if (request === folderRequest) choosingFolder = false;
    }
  }
  // The sidebar's Scan folder: pick a folder, then scan it in view (the queue page would hide it).
  async function scanFolder() {
    const chosen = await chooseFolder();
    if (!chosen) return;
    queueOpen = false;
    await start(chosen);
  }
  // Set when a disk's scan was refused because another volume now sits at its path: the next disk
  // refresh runs a full discovery even though the mounted paths look unchanged.
  let rediscover = false;
  function fail(e: unknown, context?: { title: string; reason: string }) {
    setError(String(e), context);
  }
  onMount(() => {
    let cancelled = false;
    if (isTauri()) void loadAccess(() => !cancelled);
    if (isTauri())
      invoke<Target[]>("discover_targets")
        .then((t) => {
          if (cancelled) return;
          targets = t;
          // The startup disk (first volume) is selected at launch.
          target = t.find((item) => item.volume) ?? t[0] ?? null;
        })
        .catch((e) => {
          if (!cancelled) fail(e);
        })
        .finally(() => {
          if (!cancelled) targetsLoading = false;
        });
    // Pick up disks that were connected or ejected: a cheap mount-table check, a full
    // discovery only when the set of volume paths changed.
    let refreshingDisks = false;
    async function refreshDisks() {
      // targetsLoading: the startup discovery owns the list until it settles.
      if (
        refreshingDisks ||
        targetsLoading ||
        cancelled ||
        !isTauri() ||
        performanceBuild
      )
        return;
      refreshingDisks = true;
      try {
        const mounted = await invoke<string[]>("mounted_volumes");
        const shown = targets.filter((item) => item.volume);
        if (
          cancelled ||
          (!rediscover &&
            mounted.length === shown.length &&
            shown.every((item) => mounted.includes(item.path)))
        )
          return;
        const found = (await invoke<Target[]>("discover_targets")).filter(
          (item) => item.volume,
        );
        if (cancelled || busy) return;
        // A volume still in the mount table but missing from `found` failed to register this time; it is not ejected.
        const present = new Set([
          ...found.map((item) => item.path),
          ...mounted,
        ]);
        // The backend gives a remounted or swapped volume a new id and drops the old one: the new target replaces it.
        const fresh = new Map(found.map((item) => [item.path, item]));
        const kept = targets
          .filter((item) => !item.volume || present.has(item.path))
          .map((item) => (item.volume ? (fresh.get(item.path) ?? item) : item));
        const added = found.filter(
          // Only volumes count: a chosen folder may share a disk's path.
          (item) =>
            !targets.some((old) => old.volume && old.path === item.path),
        );
        const next = [...kept, ...added];
        const startup =
          next.find((item) => item.volume && item.path === "/") ??
          next.find((item) => item.volume) ??
          next[0] ??
          null;
        const gone = targets.filter(
          (item) => item.volume && !present.has(item.path),
        );
        rediscover = false;
        targets = next;
        if (liveTarget?.volume)
          liveTarget = fresh.get(liveTarget.path) ?? liveTarget;
        dropEjected(gone, startup);
        if (target?.volume && !kept.includes(target))
          target =
            kept.find((item) => item.volume && item.path === target?.path) ??
            startup;
      } catch {
        // The next tick tries again.
      } finally {
        refreshingDisks = false;
      }
    }
    const onFocus = () => {
      if (openedSettings) void recheckAccess();
      void refreshDisks();
    };
    window.addEventListener("focus", onFocus);
    const diskTimer =
      isTauri() && !performanceBuild
        ? setInterval(() => {
            if (document.visibilityState === "visible") void refreshDisks();
          }, 3000)
        : undefined;
    return () => {
      cancelled = true;
      disposed = true;
      ++folderRequest;
      clearInterval(diskTimer);
      window.removeEventListener("focus", onFocus);
    };
  });
  const benchmark = performanceBuild
    ? createPerformanceHarness({
        get map() {
          return map;
        },
        get summary() {
          return summary;
        },
        get focus() {
          return focus;
        },
        get listLoading() {
          return listLoading;
        },
        get targets() {
          return targets;
        },
        get access() {
          return access;
        },
        get sortReady() {
          return sortReady;
        },
        get busy() {
          return busy;
        },
        get liveTarget() {
          return liveTarget;
        },
        activeId,
        setTarget: (t) => (target = t),
        start: (t) => start(t),
        setError: (message) => setError(message),
      })
    : null;
  function start(t = target) {
    if (!t) return Promise.resolve();
    // A saved scan keeps the target it started with, whose id the backend may have replaced since: scan the place
    // as it is listed now.
    const listed = targets.find(
      (x) =>
        x.path === t.path &&
        !!x.volume === !!t.volume &&
        !!x.home === !!t.home,
    );
    return scans.start(listed ?? t);
  }
  // The app's side of a start, for the scan slots: its gates, then what the view does as the scan
  // reports. Only a start that no newer start or navigation overtook (`g`) takes the view.
  function beginStart(
    t: Target,
    how: { takeView: boolean; batch: boolean },
  ): StartView | null {
    const blocking = {
      target: t,
      scanAll: how.batch ? (true as const) : undefined,
    };
    if (choosingFolder || quickLook.isPending() || scanBlocked(blocking))
      return null;
    quickLook.dismiss();
    // -1 never matches the current generation, so this start never takes the view.
    let g = how.takeView ? ++generation.current : -1;
    // Selecting or opening another place since the press leaves the view to that place.
    const pressed = { id: target?.id, home: homeOpen, queue: queueOpen };
    let taken = false;
    const samePlace = () =>
      g === generation.current && target?.id === pressed.id;
    const benchmarkClock = benchmark?.scanStarting();
    setError("");
    return {
      ready: benchmarkClock,
      // After taking the view, the start stops being `home` (accepted closes it).
      current: () =>
        (taken || (homeOpen === pressed.home && queueOpen === pressed.queue)) &&
        samePlace(),
      accepted(id, replaced) {
        // The backend just replaced the scan on screen, so its id is dead: this start's scan takes its place.
        if (
          replaced !== undefined &&
          !homeOpen &&
          !queueOpen &&
          activeId.current === replaced
        ) {
          g = generation.current;
          pressed.id = target?.id;
          pressed.home = homeOpen;
        }
        // Only the newest start takes the view; the others update their sidebar rows.
        if (
          !samePlace() ||
          homeOpen !== pressed.home ||
          queueOpen !== pressed.queue
        )
          return;
        taken = true;
        // A rejected target leaves the displayed workspace usable.
        homeOpen = false;
        queueOpen = false;
        clearResultsReturn();
        clearNavigationHistory();
        liveTarget = t;
        benchmark?.viewTaken();
        ++navigation.current;
        activeId.current = id;
        ++listRequest.current;
        summary = null;
        cancelling = false;
        trash = null;
        focus = null;
        crumbs = [];
        selection = [];
        map = [];
        page = { items: [], total: 0 };
        offset = 0;
        query = "";
        debouncedQuery = "";
        filters = { ...emptyFilter };
      },
      event(s, restarted) {
        // Progress from a scan whose results are not on screen only updates its slot.
        if (activeId.current !== s.scanId) return;
        benchmark?.progress(s.state);
        if (s.state !== "running") cancelling = false;
        if (
          s.state === "cancelled" &&
          !restarted &&
          summary?.state !== "cancelled"
        )
          showToast(
            exploring
              ? "Scan cancelled. Partial results are shown."
              : `Scan of ${liveTarget?.name ?? "the disk"} cancelled.`,
          );
        summary = s;
      },
      fetched: (fetched) => {
        // A start that never took the view (Home or the queue was open) must not replace its summary.
        if (activeId.current !== fetched.scanId) return;
        summary = fetched;
      },
      failed(e) {
        const accessOff = String(e).includes("FULL_DISK_ACCESS_OFF");
        // Scan all reports its failures together, but a disk swapped at its path still needs rediscovery.
        if (t.volume && String(e).includes("The chosen folder changed"))
          rediscover = true;
        if (how.batch && !accessOff) return false;
        // The backend probed again and found access off: keep that state even if the view moved on.
        if (accessOff)
          access = {
            status: "off",
            scanAnyway: false,
            appPath: access?.appPath ?? "",
          };
        if (!how.batch && g !== generation.current) {
          // The user is elsewhere: a toast, not a dialog over the new view.
          if (!accessOff) showToast(`Couldn’t scan ${t.name}: ${String(e)}`);
          return true;
        }
        if (accessOff) {
          // Scan all asks once.
          if (!how.batch || !blockedScan) {
            blockScan(blocking);
            void loadAccess();
          }
        } else {
          fail(e);
        }
        return true;
      },
    };
  }
  const disks = $derived(targets.filter((item) => item.volume));
  // Starts a fresh scan of every disk; one that is scanning is cancelled first. A refused start
  // does not stop the others: the failures are reported together.
  async function scanAllDisks() {
    if (!can.scanAllDisks) return;
    if (scanBlocked({ target: disks[0], scanAll: true })) return;
    // Only the start of the disk on screen (else the selected one) takes the view, and only if the user has not selected or opened
    // anything else since the press; the other starts just update their sidebar rows.
    const pressed = {
      generation: generation.current,
      id: target?.id,
      home: homeOpen,
      // The disk whose results are on screen, else the selected one.
      key:
        (exploring ? displayedSlot : null) ?? (target ? slotKey(target) : null),
    };
    const failures = await scans.scanAll(() => disks, {
      takeView: (key) =>
        key === pressed.key &&
        target?.id === pressed.id &&
        generation.current === pressed.generation &&
        homeOpen === pressed.home,
    });
    if (failures.length)
      setError(failures.join("\n"), {
        title: "Couldn’t scan every disk",
        reason:
          "Hahakigami could not start a scan on these disks. Scans on the other disks are not affected.",
      });
  }
  const scanIdentity = $derived(summary?.scanId);
  const scanEntries = $derived(summary?.entries);
  const scanState = $derived(summary?.state);
  // Entries are matched against the scan of their own disk, so any slot's scan changing can change a status.
  // Reads the deriveds, not `summary`: that is replaced on every progress event.
  $effect(() => {
    [scanIdentity, scanState, scans.signature];
    untrack(() => {
      if (!performanceBuild) void refreshQueue();
    });
  });
  const scanPhase = $derived(summary?.scanPhase);
  $effect(() => {
    [scanIdentity, scanEntries, scanState, scanPhase, focus];
    return untrack(() => {
      if (!summary || focus) return;
      let ignore = false;
      const snapshot = summary;
      invoke<NodeView>("get_node", {
        scanId: snapshot.scanId,
        nodeId: snapshot.rootId,
      })
        .then((n) => {
          // Progress events re-run this effect every ~100 ms; a reply for the same scan still applies until focus is set.
          if (!ignore || (summary?.scanId === snapshot.scanId && !focus)) {
            focus = n;
            crumbs = [n];
          }
        })
        .catch((e) => {
          if (!ignore && snapshot.state !== "running") fail(e);
        });
      return () => {
        ignore = true;
      };
    });
  });
  const revision = $derived(
    `${summary?.state}:${Math.floor((summary?.elapsedMs || 0) / 1000)}`,
  );
  const selectionIdentity = $derived(
    selection.map((node) => node.id).join(","),
  );
  $effect(() => {
    [scanIdentity, revision, selectionIdentity, exploring];
    return untrack(() => {
      if (!summary || !selection.length || !exploring) return;
      const scanId = summary.scanId,
        version = revision,
        ids = selectionIdentity;
      let ignore = false;
      const current = () =>
        !ignore &&
        summary?.scanId === scanId &&
        revision === version &&
        selectionIdentity === ids;
      // A running scan ticks every second: refresh only the newest picks, the rest at the final state.
      // A finished scan's nodes are already current: refetch only the picks that were not read from it.
      const settled = summary.state !== "running";
      const loaded = settled ? settledNodes : null;
      const nodeIds = (
        summary.state === "running" ? selection.slice(-20) : selection
      )
        .filter((node) => !loaded?.has(node))
        .map(({ id }) => id);
      if (!nodeIds.length) return;
      queryRefresh(() =>
        Promise.all(
          nodeIds.map((id) =>
            invoke<NodeView>("get_node", { scanId, nodeId: id }),
          ),
        )
          .then((nodes) => {
            if (!current()) return;
            if (settled) markSettled(nodes);
            const fresh = new Map(nodes.map((node) => [node.id, node]));
            selection = selection.map((node) => fresh.get(node.id) ?? node);
          })
          .catch((failure) => {
            if (current()) fail(failure);
          }),
      );
      return () => {
        ignore = true;
      };
    });
  });
  const focusIdentity = $derived(focus?.id);
  const mapRevision = $derived(
    mapColorMode === "fileType" && summary?.state === "running"
      ? `${summary.state}:${Math.floor(summary.elapsedMs / 2000)}`
      : revision,
  );
  // A live search or filter walks the whole tree under the scan's read lock; refresh it on user
  // changes and when the scan ends, not every second.
  const listRevision = $derived(
    summary?.state === "running" && (debouncedQuery.trim() || filtered)
      ? summary.state
      : revision,
  );
  $effect(() => {
    [scanIdentity, mapRevision, focusIdentity, mapColorMode, exploring];
    return untrack(() => {
      if (!summary || !focus || !exploring) return;
      let ignore = false;
      const mapRequest = ++mapRequestEpoch;
      benchmark?.mapQueryStarted();
      const snapshot = summary;
      const scanId = snapshot.scanId,
        nodeId = focus.id;
      invoke<NodeView>("get_node", { scanId, nodeId })
        .then((node) => {
          if (!ignore) focus = node;
        })
        .catch((failure) => {
          if (!ignore) fail(failure);
        });
      const queryColorMode = mapColorMode;
      const loadMap = async () => {
        if (ignore) return;
        await invoke<{
          nodes: NodeView[];
        }>("get_map", {
          scanId,
          nodeId,
          limit: 5000,
          ...(queryColorMode === "fileType" ? { colorMode: "fileType" } : {}),
        })
          .then((result) => {
            if (!ignore && mapRequest === mapRequestEpoch) {
              // The folder map's tile of grouped small items: the backend counts them, the count is written in the
              // user's locale here. The file-type map's leftover tile keeps its name.
              if (queryColorMode === "folder")
                for (const n of result.nodes)
                  if (n.kind === "aggregate")
                    n.name = countLabel(n.childrenCount, "smaller item");
              benchmark?.mapQueryResult(result.nodes, snapshot);
              if (snapshot.state !== "running") markSettled(result.nodes);
              map = result.nodes;
            }
          })
          .catch((failure) => {
            if (!ignore) fail(failure);
          });
      };
      if (queryColorMode === "fileType") queryFileTypeMap(loadMap);
      else queryMap(loadMap);
      return () => {
        ignore = true;
        if (mapRequest === mapRequestEpoch) {
          ++mapRequestEpoch;
          benchmark?.mapQueryEnded();
        }
      };
    });
  });
  const navigationRevision = $derived(navigation.current);
  $effect(() => {
    [
      scanIdentity,
      listRevision,
      focusIdentity,
      offset,
      query,
      debouncedQuery,
      filterKey,
      sort,
      sortReady,
      navigationRevision,
      exploring,
    ];
    return untrack(() => {
      const request = ++listRequest.current;
      if (!summary || !focus || !sortReady || !exploring) return;
      listLoading = true;
      // Preserve valid rows during background scan revisions; user changes
      // synchronously clear rows through invalidateList before requesting again.
      // Raw text changes invalidate prior results immediately, before debounce.
      if (query !== debouncedQuery) return;
      let ignore = false;
      const scanId = summary.scanId;
      const settled = summary.state !== "running";
      const current = () =>
        !ignore &&
        request === listRequest.current &&
        activeId.current === scanId;
      const common = {
        scanId: summary.scanId,
        offset,
        limit: PAGE_SIZE,
        sort,
      };
      const nameQuery = debouncedQuery.trim();
      const [command, args] = filtered
        ? ["filter_nodes", { ...common, query: nameQuery, ...filters }]
        : nameQuery
          ? ["search_nodes", { ...common, query: nameQuery }]
          : ["get_children", { ...common, nodeId: focus.id }];
      // One request at a time: overlapping searches would each hold the scan's
      // read lock. A superseded job never starts, and the newest job ends loading.
      queryList(async () => {
        if (!current()) return;
        await invoke<Page>(command, args)
          .then((result) => {
            if (!current()) return;
            const lastOffset = result.total
              ? Math.floor((result.total - 1) / PAGE_SIZE) * PAGE_SIZE
              : 0;
            if (offset > lastOffset) {
              // A growing directory can leave a maximum-size filter during a
              // live refresh. Request the last valid page without an inverted range.
              quickLook.dismiss();
              ++listRequest.current;
              page = { items: [], total: result.total };
              offset = lastOffset;
              return;
            }
            page = result;
            if (settled) markSettled(result.items);
          })
          .catch((failure) => {
            if (current()) fail(failure);
          })
          .finally(() => {
            if (current()) listLoading = false;
          });
      });
      return () => {
        ignore = true;
      };
    });
  });
  function captureView(): NavigationView | null {
    if (!summary || !focus || activeId.current !== summary.scanId) return null;
    return {
      scanId: summary.scanId,
      focus,
      // A new focus can precede completion of its breadcrumb lookup.
      crumbs:
        crumbs[0]?.id === summary.rootId && crumbs.at(-1)?.id === focus.id
          ? [...crumbs]
          : [],
      query,
      filters: { ...filters },
      sort,
      offset,
      selection: [...selection],
      total: page.total,
      scroll: listScroll,
      resultsReturn,
    };
  }
  function rememberView(view: NavigationView) {
    // Bound retained selections and result contexts during long browsing sessions.
    backHistory = [...backHistory.slice(-99), view];
    forwardHistory = [];
  }
  /** `known` is the current breadcrumb chain of the same scan: the walk up stops at the first folder in it. */
  async function loadCrumbs(
    n: NodeView,
    scanId: string,
    nav: number,
    known: readonly NodeView[] = [],
  ) {
    const chain: NodeView[] = [n];
    let id = n.parentId;
    try {
      while (id !== null) {
        const at = known.findIndex((crumb) => crumb.id === id);
        if (at >= 0) {
          chain.unshift(...known.slice(0, at + 1));
          break;
        }
        const parent = await invoke<NodeView>("get_node", {
          scanId,
          nodeId: id,
        });
        chain.unshift(parent);
        id = parent.parentId;
      }
      if (activeId.current === scanId && navigation.current === nav)
        crumbs = chain;
    } catch (e) {
      if (activeId.current === scanId && navigation.current === nav) fail(e);
    }
  }
  async function restoreView(
    snapshot: ResultsReturn,
    returnPoint: ResultsReturn | null,
  ) {
    const nav = ++navigation.current;
    invalidateList();
    focus = snapshot.focus;
    crumbs = snapshot.crumbs.length ? [...snapshot.crumbs] : [snapshot.focus];
    query = snapshot.query;
    debouncedQuery = snapshot.query;
    filters = { ...snapshot.filters };
    const sortChanged = sort !== snapshot.sort;
    ++sortChoice.current;
    sort = snapshot.sort;
    offset = snapshot.offset;
    selection = [...snapshot.selection];
    listScroll = snapshot.scroll;
    listScrollRestore = { id: ++scrollRestoreId, top: snapshot.scroll };
    resultsReturn = returnPoint;
    if (sortChanged) void persistSort(snapshot.sort);
    if (!snapshot.crumbs.length)
      await loadCrumbs(snapshot.focus, snapshot.scanId, nav);
  }
  function canNavigateHistory(history: NavigationView[]) {
    return !!(
      summary &&
      focus &&
      activeId.current === summary.scanId &&
      history.at(-1)?.scanId === summary.scanId &&
      !sessionBlocked &&
      !previewDialog &&
      !homeOpen &&
      !queueOpen
    );
  }
  async function goBack() {
    if (!canNavigateHistory(backHistory)) return;
    const current = captureView()!;
    const snapshot = backHistory.at(-1)!;
    backHistory = backHistory.slice(0, -1);
    forwardHistory = [...forwardHistory.slice(-99), current];
    await restoreView(snapshot, snapshot.resultsReturn);
  }
  async function goForward() {
    if (!canNavigateHistory(forwardHistory)) return;
    const current = captureView()!;
    const snapshot = forwardHistory.at(-1)!;
    forwardHistory = forwardHistory.slice(0, -1);
    backHistory = [...backHistory.slice(-99), current];
    await restoreView(snapshot, snapshot.resultsReturn);
  }
  async function open(n: NodeView) {
    if (n.excluded) {
      select(n);
      return;
    }
    if (
      n.kind !== "directory" ||
      !summary ||
      !focus ||
      activeId.current !== summary.scanId ||
      sessionBlocked ||
      previewDialog
    )
      return;
    const previous = captureView()!;
    if (n.id !== focus.id || query.trim() || filtered) rememberView(previous);
    if (!resultsReturn && (query.trim() || filtered)) resultsReturn = previous;
    quickLook.dismiss();
    const scanId = summary.scanId;
    const nav = ++navigation.current;
    invalidateList();
    focus = n;
    offset = 0;
    query = "";
    debouncedQuery = "";
    filters = { ...emptyFilter };
    ++listRequest.current;
    selection = [n];
    // Crumbs still hold the previous folder's chain; opening a child (the usual case) needs no backend call.
    // Only a chain from the root is reused: a restore can leave just the focus while its own walk is pending.
    await loadCrumbs(
      n,
      scanId,
      nav,
      crumbs[0]?.parentId === null ? crumbs : [],
    );
  }
  function returnToResults() {
    const snapshot = resultsReturn;
    if (
      !snapshot ||
      !summary ||
      homeOpen ||
      queueOpen ||
      snapshot.scanId !== summary.scanId ||
      activeId.current !== snapshot.scanId ||
      sessionBlocked ||
      previewDialog
    )
      return;
    const current = captureView();
    if (current) rememberView(current);
    void restoreView(snapshot, null);
  }
  function selectRange(nodes: NodeView[]) {
    nodes = nodes.filter((n) => n.kind !== "aggregate");
    if (!nodes.length) return;
    if (
      nodes.length > 1 ||
      selection.length !== 1 ||
      selection[0].id !== nodes[0].id
    )
      quickLook.dismiss();
    selection = nodes;
  }
  /** The rows inside an expanded folder of the list. */
  async function loadBranch(nodeId: number) {
    if (!summary) throw new Error("No scan to read");
    const scanId = summary.scanId,
      settled = summary.state !== "running";
    const branch = await invoke<Page>("get_children", {
      scanId,
      nodeId,
      offset: 0,
      limit: PAGE_SIZE,
      sort,
    });
    if (settled) markSettled(branch.items);
    return branch;
  }
  function select(n: NodeView, multi = false) {
    if (n.kind === "aggregate") return;
    if (multi || selection.length !== 1 || selection[0].id !== n.id)
      quickLook.dismiss();
    selection = multi
      ? selection.some((x) => x.id === n.id)
        ? selection.filter((x) => x.id !== n.id)
        : [...selection, n]
      : [n];
  }
  async function prepareTrash() {
    if (
      !summary ||
      summary.state !== "completed" ||
      !selection.length ||
      sessionBlocked ||
      quickLook.isPending() ||
      selection.some((node) => node.excluded)
    )
      return;
    quickLook.dismiss();
    busy = true;
    const scanId = summary.scanId;
    try {
      const review = await invoke<{ token: string; items: NodeView[] }>(
        "prepare_trash",
        { scanId, nodeIds: selection.map((n) => n.id) },
      );
      if (summary?.scanId === scanId) trash = { ...review, scanId };
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
  async function commitTrash() {
    if (!summary || busy || !trash) return;
    const review = {
      ...trash,
      items: trash.items.map((item) => ({ ...item })),
    };
    const scanId = review.scanId;
    const rescanTarget = liveTarget;
    const invalidateAndRescan = () => {
      clearResultsReturn();
      clearNavigationHistory();
      // A cleanup that moved anything invalidates these results before a replacement scan starts.
      scans.drop(scanId);
      summary = null;
      void start(rescanTarget);
    };
    busy = true;
    try {
      const result = await invoke<{ staleScanIds?: string[] } | null>(
        review.queued ? "commit_cleanup_queue" : "commit_trash",
        {
          scanId,
          token: review.token,
        },
      );
      // Other scans whose root overlaps a moved item; the backend already invalidated them.
      const stale = result?.staleScanIds;
      if (Array.isArray(stale)) for (const id of stale) scans.drop(id);
      const report = cleanupResult(
        ++cleanupReportId,
        review.queued ? "queue" : "selection",
        review.items,
        result,
      );
      cleanupReport = report;
      trash = null;
      void refreshQueue();
      // An unconfirmed reply may still have moved files, so it invalidates too.
      if (
        report.movedCount ||
        report.invalidated ||
        report.state !== "confirmed"
      )
        invalidateAndRescan();
    } catch (e) {
      trash = null;
      const base = cleanupResult(
        ++cleanupReportId,
        review.queued ? "queue" : "selection",
        review.items,
        null,
      );
      // The backend refused before moving anything: a plain failure, not an unconfirmed result.
      if (String(e).startsWith("Nothing was moved: ")) {
        cleanupReport = {
          ...base,
          state: "confirmed",
          movedCount: 0,
          failedCount: review.items.length,
          message: String(e),
        };
      } else {
        cleanupReport = {
          ...base,
          message: `Cleanup could not be confirmed. ${String(e)} Check the items in Finder before trying again.`,
        };
        // A crashed backend task may have moved files before failing, so these results are stale.
        invalidateAndRescan();
      }
      void refreshQueue();
    } finally {
      busy = false;
    }
  }
  const inspected = $derived(selection[selection.length - 1] || focus);
  const parentFolder = $derived(
    summary &&
      focus &&
      focus.id !== summary.rootId &&
      crumbs[0]?.id === summary.rootId
      ? crumbs.find((node) => node.id === focus?.parentId)
      : undefined,
  );
  // A selected target describes the next scan, never the identity of existing results.
  const resultLocation = $derived.by(() => {
    if (!summary) return null;
    const root = crumbs[0];
    const scanned = liveTarget;
    const path = scanned?.path ?? root?.path;
    return path ? { name: scanned?.name ?? root?.name, path } : null;
  });
  const displayedTargetSelected = $derived(
    !!summary && !!resultLocation && target?.path === resultLocation.path,
  );
  // Scan the location of the results on screen, whatever the sidebar has selected.
  function rescanDisplayed() {
    return start(liveTarget);
  }
  function rescan() {
    return displayedTargetSelected && !homeOpen
      ? rescanDisplayed()
      : start(target);
  }
  function showHome() {
    quickLook.dismiss();
    queueOpen = false;
    homeOpen = true;
  }
  // Leave Home or the queue for the results already loaded.
  function showResults() {
    quickLook.dismiss();
    queueOpen = false;
    homeOpen = false;
  }
  // Open a slot's own scan; nothing is reloaded or rescanned.
  function showSlot(key: string) {
    const slot = scans.slots[key];
    if (!slot) return;
    const onScreen = summary
      ? summary.scanId === slot.scanId
      : !slot.summary && activeId.current === slot.scanId;
    if (!onScreen) {
      liveTarget = slot.target;
      resetView(slot.summary, slot.scanId);
    }
    showResults();
  }
  // A sidebar disk or the folder row: one with a scan shows it and is selected, so the toolbar rescans it.
  // A disk without a scan opens the welcome page with it selected.
  function openDisk(place: Target) {
    if (sessionBlocked) return;
    target = place;
    if (scans.slots[slotKey(place)]) showSlot(slotKey(place));
    else showHome();
  }
  // An ejected disk's scan is freed in the backend and dropped here. If its results were on screen
  // the welcome page opens with the startup disk selected.
  function dropEjected(gone: Target[], startup: Target | null) {
    const wasShown = exploring;
    let shown = false;
    for (const disk of gone) {
      const scanId = scans.discard(disk);
      shown ||= scanId !== null && activeId.current === scanId;
    }
    if (!shown) return;
    resetView(null, null);
    liveTarget = null;
    if (wasShown) target = startup;
    void refreshQueue();
  }
  // The explorer (map, file list, inspector) is what the user is looking at.
  const exploring = $derived(!!summary && !homeOpen && !queueOpen);
  const selectionActionable = $derived(
    exploring &&
      summary?.state === "completed" &&
      selection.length > 0 &&
      !sessionBlocked &&
      !selection.some((node) => node.excluded),
  );
  // The backend refuses to review more than this many items for Trash.
  const trashActionable = $derived(
    selectionActionable && selection.length <= MAX_TRASH_ITEMS,
  );
  const readyHereEntries = $derived(cleanupQueue.readyHere);
  const queueSelection = cleanupQueue.selection;
  /**
   * The whole availability answer for commands, shared by buttons and the menu bar: App.svelte maps
   * it straight through. Open dialogs and popovers only quiet the menu as a whole (menuQuiet).
   */
  const can = {
    get goBack() {
      return canNavigateHistory(backHistory);
    },
    get goForward() {
      return canNavigateHistory(forwardHistory);
    },
    get enclosing() {
      return (
        !!parentFolder &&
        !homeOpen &&
        !sessionBlocked &&
        !previewDialog &&
        !queueOpen
      );
    },
    get openFolder() {
      const node = selection.length === 1 ? selection[0] : null;
      return (
        exploring &&
        node?.kind === "directory" &&
        !node.excluded &&
        !sessionBlocked &&
        !previewDialog
      );
    },
    get trash() {
      return trashActionable;
    },
    // The add button only exists for an unqueued selection; the menu item needs that spelled out.
    get addToQueue() {
      return (
        selectionActionable &&
        !queue.blocked &&
        !queueSelection.queued &&
        !queueSelection.directlyQueued &&
        !queueSelection.adding &&
        !queueSelection.removing
      );
    },
    /** Review the ready entries matched to the scan on screen. */
    get reviewHere() {
      return (
        !sessionBlocked &&
        !queue.blocked &&
        shown?.summary.state === "completed" &&
        readyHereEntries.length > 0
      );
    },
    get reveal() {
      return (
        exploring &&
        selection.length <= 1 &&
        !!inspected &&
        inspected.kind !== "aggregate"
      );
    },
    get quickLook() {
      return (
        exploring &&
        !quickLook.pending &&
        !!previewNode &&
        previewEligible(previewNode)
      );
    },
    // Header button with a scan shown; the welcome button when there is none.
    get rescan() {
      const shown = !!summary && !homeOpen;
      const onDisplayed = shown && displayedTargetSelected;
      // What the click scans: the displayed results' place or the sidebar target. Only that slot
      // has to be free; other disks may be scanning.
      const place = onDisplayed ? liveTarget : target;
      return (
        !!place &&
        !sessionBlocked &&
        !scans.busy(place) &&
        !!access &&
        (shown || (isTauri() && !targetsLoading))
      );
    },
    // Header and welcome-page buttons. Like rescan, minus the per-slot check: busy disks are restarted.
    get scanAllDisks() {
      return (
        isTauri() &&
        !targetsLoading &&
        !!access &&
        !sessionBlocked &&
        !scans.scanningAll &&
        disks.length > 0
      );
    },
    // Dialogs about the results on screen: they rescan those, not the sidebar target.
    get rescanDisplayed() {
      return (
        !!summary &&
        !homeOpen &&
        !!liveTarget &&
        !sessionBlocked &&
        !scans.busy(liveTarget) &&
        !!access
      );
    },
    get stopScan() {
      return (
        !!summary && running && !cancelling && summary.scanPhase !== "caching"
      );
    },
    get chooseFolder() {
      return isTauri() && !sessionBlocked && !targetsLoading && !!access;
    },
    // A new pick replaces the one folder slot, so a running folder scan is never cancelled by it.
    get scanFolder() {
      return can.chooseFolder && !scans.folderBusy();
    },
    get cleanupQueue() {
      return !blocked && !trash && !queueBusy;
    },
    get find() {
      return exploring;
    },
    get explore() {
      return exploring;
    },
    get toggleSidebar() {
      return (
        !accessOpen && !trash && !clearQueueOpen && !cleanupReport && !error
      );
    },
    get toggleFileList() {
      return exploring;
    },
  };
  return {
    can,
    get readyHereEntries() {
      return readyHereEntries;
    },
    queueSelection,
    get parentFolder() {
      return parentFolder;
    },
    get resultLocation() {
      return resultLocation;
    },
    get displayedTargetSelected() {
      return displayedTargetSelected;
    },
    rescan,
    rescanDisplayed,
    scanAllDisks,
    get batchProgress() {
      return scans.batchProgress;
    },
    goBack,
    goForward,
    get resultsReturn() {
      return resultsReturn;
    },
    get listScrollRestore() {
      return listScrollRestore;
    },
    setListScroll,
    finishListScrollRestore,
    changeQuery,
    returnToResults,
    get cleanupReport() {
      return cleanupReport;
    },
    dismissCleanupReport,
    get queue() {
      return queue;
    },
    get homeOpen() {
      return homeOpen;
    },
    openDisk,
    get scanSlots() {
      return scans.slots;
    },
    get shown() {
      return shown;
    },
    get queueOpen() {
      return queueOpen;
    },
    get queueBusy() {
      return queueBusy;
    },
    get queueTargeting() {
      return cleanupQueue.targeting;
    },
    get queueError() {
      return cleanupQueue.error;
    },
    cleanupQueue,
    get clearQueueOpen() {
      return clearQueueOpen;
    },
    showQueue,
    scanQueue,
    prepareQueue,
    setClearQueueOpen: (value: boolean) => {
      clearQueueOpen = value;
    },
    performanceLayoutPending: () => benchmark?.layoutPending(),
    get performanceMapReady() {
      return benchmark?.mapReady ?? false;
    },
    get targets() {
      return targets;
    },
    get target() {
      return target;
    },
    get summary() {
      return summary;
    },
    get revision() {
      return revision;
    },
    get focus() {
      return focus;
    },
    get crumbs() {
      return crumbs;
    },
    get page() {
      return page;
    },
    get map() {
      return map;
    },
    get mapColorMode() {
      return mapColorMode;
    },
    changeMapColorMode,
    get selection() {
      return selection;
    },
    get mode() {
      return mode;
    },
    get query() {
      return query;
    },
    get offset() {
      return offset;
    },
    get error() {
      return error;
    },
    get errorContext() {
      return errorContext;
    },
    get busy() {
      return busy;
    },
    get cancelling() {
      return cancelling;
    },
    get trash() {
      return trash;
    },
    get filters() {
      return filters;
    },
    get debouncedQuery() {
      return debouncedQuery;
    },
    get listLoading() {
      return listLoading;
    },
    get sort() {
      return sort;
    },
    get sortReady() {
      return sortReady;
    },
    get sortSaving() {
      return sortSaving;
    },
    get sortWarning() {
      return sortWarning;
    },
    get access() {
      return access;
    },
    get accessOpen() {
      return accessOpen;
    },
    get accessPrompt() {
      return accessPrompt;
    },
    get accessBusy() {
      return accessBusy;
    },
    get accessError() {
      return accessError;
    },
    get accessReturned() {
      return accessReturned;
    },
    loadBranch,
    setTarget: (value: Target | null) => {
      target = value;
    },
    setMode: (value: "treemap" | "sunburst") => {
      mode = value;
    },
    setOffset: (value: number) => {
      offset = value;
    },
    get setError() {
      return setError;
    },
    setCancelling: (value: boolean) => {
      cancelling = value;
    },
    setTrash: (value: typeof trash) => {
      trash = value;
    },
    setAccessError: (value: string) => {
      accessError = value;
    },
    get quickLook() {
      return quickLook;
    },
    get filtered() {
      return filtered;
    },
    get filterKey() {
      return filterKey;
    },
    get blocked() {
      return blocked;
    },
    get running() {
      return running;
    },
    get anyScanRunning() {
      return scans.anyRunning;
    },
    // A queue scan of this root would use a slot that is already scanning: its disk's, else the folder slot.
    queueRootBusy(path: string) {
      const disk = listedDisk(path);
      return disk ? scans.busy(disk) : scans.folderBusy();
    },
    /** A start of this place has not heard back from the backend yet. */
    scanStarting: (place: Target) => scans.starting(place),
    get sessionBlocked() {
      return sessionBlocked;
    },
    get previewDialog() {
      return previewDialog;
    },
    get previewNode() {
      return previewNode;
    },
    get previewVisible() {
      return previewVisible;
    },
    get inspected() {
      return inspected;
    },
    get navigation() {
      return navigation;
    },
    get liveTarget() {
      return liveTarget;
    },
    get activeId() {
      return activeId;
    },
    get invalidateList() {
      return invalidateList;
    },
    get changeFilters() {
      return changeFilters;
    },
    get persistSort() {
      return persistSort;
    },
    get changeSort() {
      return changeSort;
    },
    get previewEligible() {
      return previewEligible;
    },
    get previewRow() {
      return previewRow;
    },
    get toast() {
      return toast;
    },
    dismissToast,
    get openAccess() {
      return openAccess;
    },
    get closeAccess() {
      return closeAccess;
    },
    get scanAnyway() {
      return scanAnyway;
    },
    get openSettings() {
      return openSettings;
    },
    get relaunch() {
      return relaunch;
    },
    get chooseFolder() {
      return chooseFolder;
    },
    get scanFolder() {
      return scanFolder;
    },
    get choosingFolder() {
      return choosingFolder;
    },
    get targetsLoading() {
      return targetsLoading;
    },
    get fail() {
      return fail;
    },
    get performanceRendered() {
      return (nodes: NodeView[]) => benchmark?.rendered(nodes);
    },
    get start() {
      return start;
    },
    get open() {
      return open;
    },
    get selectRange() {
      return selectRange;
    },
    get select() {
      return select;
    },
    get prepareTrash() {
      return prepareTrash;
    },
    get commitTrash() {
      return commitTrash;
    },
  };
}
