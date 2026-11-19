import { onMount, untrack } from "svelte";
import createQuickLook from "./createQuickLook.svelte";
import { createCleanupQueue, tauriQueuePort } from "./cleanupQueue.svelte";
import {
  createSessionState,
  createExplorerState,
  rawState,
} from "./appState.svelte";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
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
import type { CleanupReport, QueueView } from "./cleanupTypes";
import { cleanupResult } from "./cleanupResults";
import type { AppToast } from "./toastTypes";
import { canvasEvidence, runNativeInteractions } from "./nativeBenchmark";
import {
  type FullDiskAccess,
  type NodeView,
  type Page,
  type ScanSummary,
  type Target,
} from "./types";
const performanceBuild = import.meta.env.VITE_PERFORMANCE_BENCHMARK === "1";
declare global {
  interface Window {
    __PERFORMANCE_BENCHMARK__?: {
      root: string;
      mode: string;
      interactions?: boolean;
    };
  }
}
export default function createApp() {
  const session = createSessionState(),
    explorer = createExplorerState();
  const performanceConfigState = rawState<{
    root: string;
    mode: string;
    interactions?: boolean;
  } | null>(null);
  const setPerformanceConfig = performanceConfigState.set;
  let performanceConfig = $derived(performanceConfigState.value);
  const performanceStarted = { current: false };
  const performanceReported = { current: false };
  const performanceRescanned = { current: false };
  let performanceDrawnMap = $state.raw<NodeView[] | null>(null);
  let performanceProgressUpdates = 0;
  let performanceMapReady = $state.raw(false);
  let performanceMapQueryPending = $state.raw(false);
  let performanceMapQueryEpoch = 0;
  const performanceEventsReadyState = rawState(!performanceBuild);
  const setPerformanceEventsReady = performanceEventsReadyState.set;
  let performanceEventsReady = $derived(performanceEventsReadyState.value);
  const performanceMapScanState = rawState<string | null>(null);
  const setPerformanceMapScan = performanceMapScanState.set;
  let performanceMapScan = $derived(performanceMapScanState.value);
  onMount(() => {
    if (!performanceBuild) return;
    const receive = () =>
      setPerformanceConfig(window.__PERFORMANCE_BENCHMARK__ ?? null);
    receive();
    window.addEventListener("performance-config", receive);
    return () => window.removeEventListener("performance-config", receive);
  });
  const setTargets = session.setTargets;
  let targets = $derived(session.targets);
  const setTarget = session.setTarget;
  let target = $derived(session.target);
  const setSummary = session.setSummary;
  let summary = $derived(session.summary);
  const setFocus = explorer.setFocus;
  let focus = $derived(explorer.focus);
  const setCrumbs = explorer.setCrumbs;
  let crumbs = $derived(explorer.crumbs);
  const setPage = explorer.setPage;
  let page = $derived(explorer.page);
  const setMap = explorer.setMap;
  let map = $derived(explorer.map);
  let mapColorMode = $state<"folder" | "fileType">("folder");
  const queryFileTypeMap = latestOnly();
  const queryMap = latestOnly();
  const queryRefresh = latestOnly();
  const queryList = latestOnly();
  function changeMapColorMode(value: "folder" | "fileType") {
    mapColorMode = value;
  }
  const setSelection = explorer.setSelection;
  let selection = $derived(explorer.selection);
  const setMode = explorer.setMode;
  let mode = $derived(explorer.mode);
  const setQuery = explorer.setQuery;
  let query = $derived(explorer.query);
  const setOffset = explorer.setOffset;
  let offset = $derived(explorer.offset);
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
  const opBusy = rawState(false);
  const setBusy = opBusy.set;
  const busy = $derived(opBusy.value);
  const setCancelling = session.setCancelling;
  let cancelling = $derived(session.cancelling);
  const trashState = rawState<{
    // The scan the review was made for; commit must target it, not whatever is on screen later.
    scanId: string;
    queued?: boolean;
    token: string;
    items: NodeView[];
  } | null>(null);
  const setTrash = trashState.set;
  let trash = $derived(trashState.value);
  let queueOpen = $state(false);
  // Home shows the welcome page without discarding the loaded or running scan.
  let homeOpen = $state(false);
  let cleanupOutcome = $state("");
  let cleanupReport = $state.raw<CleanupReport | null>(null);
  let cleanupReportId = 0;
  function dismissCleanupReport() {
    cleanupReport = null;
    cleanupOutcome = "";
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
      if (trash?.queued) setTrash(null);
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
    if (busy || trash || queueBusy || choosingFolder) return;
    quickLook.dismiss();
    queueOpen = value;
    if (value) {
      homeOpen = false;
      void refreshQueue();
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
  function scanQueue(entryId: string) {
    if (sessionBlocked) return;
    return cleanupQueue.scan(entryId, async (location) => {
      // A disk is already in the Disks list; its rescan uses the same slot, so it gets no second row.
      const disk = location.volume ? listedDisk(location.path) : undefined;
      if (!disk)
        setTargets((old) => [
          // The backend gives a recreated folder a new id and drops the old one.
          ...old.filter(
            (t) =>
              t.id !== location.id && (t.volume || t.path !== location.path),
          ),
          location,
        ]);
      setTarget(disk ?? location);
      queueOpen = false;
      await start(location);
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
    setBusy(true);
    const scanId = summary.scanId;
    try {
      const review = await cleanupQueue.prepare(scanId);
      if (summary?.scanId === scanId) setTrash({ ...review, scanId, queued: true });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }
  const quickLook = createQuickLook();
  const setFilters = explorer.setFilters;
  let filters = $derived(explorer.filters);
  const setDebouncedQuery = explorer.setDebouncedQuery;
  let debouncedQuery = $derived(explorer.debouncedQuery);
  const setListLoading = explorer.setListLoading;
  let listLoading = $derived(explorer.listLoading);
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
    setPage({ items: [], total: 0 });
    setListLoading(true);
  }
  function changeFilters(value: ExplorerFilter) {
    if (JSON.stringify(value) === filterKey) return;
    clearResultsReturn();
    invalidateList();
    setFilters(value);
    setOffset(0);
    setSelection([]);
  }
  function changeQuery(value: string) {
    if (value === query) return;
    clearResultsReturn();
    invalidateList();
    setQuery(value);
    setOffset(0);
    setSelection([]);
  }
  $effect(() => {
    [query];
    return untrack(() => {
      const timer = setTimeout(() => setDebouncedQuery(query), 250);
      return () => clearTimeout(timer);
    });
  });
  const setSort = explorer.setSort;
  let sort = $derived(explorer.sort);
  const setSortReady = explorer.setSortReady;
  let sortReady = $derived(explorer.sortReady);
  const setSortSaving = explorer.setSortSaving;
  let sortSaving = $derived(explorer.sortSaving);
  const setSortWarning = explorer.setSortWarning;
  let sortWarning = $derived(explorer.sortWarning);
  const sortLock = { current: false };
  const sortEpoch = { current: 0 };
  const sortChoice = { current: 0 };
  onMount(() => {
    const epoch = ++sortEpoch.current,
      choice = sortChoice.current;
    let cancelled = false;
    const current = () =>
      !cancelled &&
      sortEpoch.current === epoch &&
      sortChoice.current === choice;
    if (!isTauri()) {
      setSortReady(true);
      return;
    }
    invoke<ExplorerPreferences>("get_explorer_preferences")
      .then((preferences) => {
        if (!current()) return;
        if (!preferences || !isSortOrder(preferences.sort)) {
          setSortWarning(
            "Could not load the saved sort order. Using largest first.",
          );
        } else {
          setSort(preferences.sort);
          setSortWarning(preferences.warning ?? null);
        }
        setSortReady(true);
      })
      .catch((failure) => {
        if (!current()) return;
        setSortWarning(
          `Could not load the saved sort order. Using largest first. ${String(failure)}`,
        );
        setSortReady(true);
      });
    return () => {
      cancelled = true;
      ++sortEpoch.current;
    };
  });
  async function persistSort(order: SortOrder) {
    if (sortLock.current || !sortReady) return;
    sortLock.current = true;
    setSortSaving(true);
    const epoch = sortEpoch.current;
    try {
      const preferences = await invoke<ExplorerPreferences>(
        "set_explorer_sort",
        { sort: order },
      );
      if (!preferences || preferences.sort !== order)
        throw new Error("Invalid preference response");
      if (epoch === sortEpoch.current && sort === order)
        setSortWarning(preferences.warning ?? null);
    } catch (failure) {
      if (epoch === sortEpoch.current && sort === order)
        setSortWarning(
          `Sort order is on now, but it couldn't be saved. ${String(failure)}`,
        );
    } finally {
      sortLock.current = false;
      if (epoch === sortEpoch.current) {
        setSortSaving(false);
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
    setSort(order);
    setOffset(0);
    setSelection([]);
    void persistSort(order);
  }
  const accessState = rawState<FullDiskAccess | null>(null);
  const setAccess = accessState.set;
  let access = $derived(accessState.value);
  const accessOpenState = rawState(false);
  const setAccessOpen = accessOpenState.set;
  let accessOpen = $derived(accessOpenState.value);
  const accessBusyState = rawState(false);
  const setAccessBusy = accessBusyState.set;
  let accessBusy = $derived(accessBusyState.value);
  const accessErrorState = rawState("");
  const setAccessError = accessErrorState.set;
  let accessError = $derived(accessErrorState.value);
  // The user came back from System Settings and Full Disk Access is still off.
  let accessReturned = $state.raw(false);
  // The dialog was opened because a scan is waiting on Full Disk Access.
  let accessPrompt = $state.raw(false);
  let openedSettings = false;
  let blockedScan: {
    target: Target | null;
    // The blocked request was Scan all disks: "Scan anyway" runs it again for every disk.
    scanAll?: true;
  } | null =
    null;
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
    setSelection([node]);
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
    setSummary(s);
    setFocus(null);
    setCrumbs([]);
    setSelection([]);
    setMap([]);
    setPage({ items: [], total: 0 });
    setOffset(0);
    setQuery("");
    setDebouncedQuery("");
    setFilters({ ...emptyFilter });
    ++listRequest.current;
    setTrash(null);
    setCancelling(false);
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
    setAccessBusy(true);
    setAccessError("");
    try {
      const info = await invoke<FullDiskAccess>("full_disk_access_status");
      if (current()) setAccess(info);
      return info;
    } catch (e) {
      if (current()) setAccessError(String(e));
    } finally {
      if (current()) setAccessBusy(false);
    }
  }
  function openAccess() {
    quickLook.dismiss();
    setAccessError("");
    setAccessOpen(true);
    void loadAccess();
  }
  function closeAccess() {
    blockedScan = null;
    accessPrompt = false;
    openedSettings = false;
    accessReturned = false;
    setAccessError("");
    setAccessOpen(false);
  }
  // Only starting a scan waits on Full Disk Access; unknown never blocks.
  function blockScan(request: NonNullable<typeof blockedScan>) {
    blockedScan = request;
    accessPrompt = true;
    openedSettings = false;
    accessReturned = false;
    setAccessOpen(true);
  }
  function scanBlocked(request: NonNullable<typeof blockedScan>) {
    if (access?.status !== "off" || access.scanAnyway) return false;
    blockScan(request);
    return true;
  }
  async function scanAnyway() {
    const request = blockedScan;
    setAccessBusy(true);
    setAccessError("");
    try {
      setAccess(await invoke<FullDiskAccess>("scan_without_full_disk_access"));
    } catch (e) {
      setAccessError(String(e));
      return;
    } finally {
      setAccessBusy(false);
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
      setAccessError(String(e));
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
    setAccessBusy(true);
    setAccessError("");
    try {
      await invoke("restart_app");
    } catch (e) {
      setAccessError(String(e));
    } finally {
      setAccessBusy(false);
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
    const g = generation.current,
      nav = navigation.current;
    const scanId = activeId.current,
      previousTarget = target,
      previousSummary = summary;
    const current = () =>
      !disposed &&
      request === folderRequest &&
      generation.current === g &&
      navigation.current === nav &&
      activeId.current === scanId &&
      target === previousTarget &&
      // Progress replaces the summary object of a scan that keeps running while the dialog is open.
      summary?.scanId === previousSummary?.scanId;
    quickLook.dismiss();
    try {
      const chosen = await invoke<Target | null>("choose_folder");
      if (!current()) return;
      if (chosen) {
        setTarget(chosen);
        setTargets((previous) =>
          previous.some((item) => item.id === chosen.id)
            ? previous
            : [
                ...previous.filter(
                  (item) => item.volume || item.path !== chosen.path,
                ),
                chosen,
              ],
        );
      }
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
          setTargets(t);
          // The startup disk (first volume) is selected at launch.
          setTarget(t.find((item) => item.volume) ?? t[0] ?? null);
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
        // A scan starting now owns the view; the next tick retries.
        if (cancelled || busy || scans.starting) return;
        // A volume still in the mount table but missing from `found` failed to register this time; it is not ejected.
        const present = new Set([...found.map((item) => item.path), ...mounted]);
        // A refused scan means another volume now sits at the path: it replaces the old target, so a scan uses its new id.
        const fresh = new Map(found.map((item) => [item.path, item]));
        const kept = targets
          .filter((item) => !item.volume || present.has(item.path))
          .map((item) =>
            rediscover && item.volume ? (fresh.get(item.path) ?? item) : item,
          );
        const added = found.filter(
          // Only volumes count: a chosen folder may share a disk's path.
          (item) => !targets.some((old) => old.volume && old.path === item.path),
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
        const swapped = rediscover;
        rediscover = false;
        setTargets(next);
        if (swapped && liveTarget?.volume)
          liveTarget = fresh.get(liveTarget.path) ?? liveTarget;
        dropEjected(gone, startup);
        if (target?.volume && !kept.includes(target))
          setTarget(
            kept.find((item) => item.volume && item.path === target?.path) ??
              startup,
          );
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
  onMount(() => {
    if (!performanceBuild) return;
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    void listen<{
      scanId: string;
    }>("performance-rescan-ready", ({ payload }) => {
      if (
        cancelled ||
        performanceRescanned.current ||
        !["same_process", "changed"].includes(performanceConfig?.mode ?? "") ||
        !performanceReported.current ||
        activeId.current !== payload.scanId
      )
        return;
      performanceRescanned.current = true;
      activeId.current = null;
      performanceReported.current = false;
      void start(liveTarget);
    })
      .then((off) => {
        if (cancelled) off();
        else {
          unsubscribe = off;
          setPerformanceEventsReady(true);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  });
  $effect(() => {
    [
      performanceConfig,
      performanceEventsReady,
      targets,
      access,
      sortReady,
      busy,
    ];
    return untrack(() => {
      if (
        !performanceBuild ||
        !performanceEventsReady ||
        !performanceConfig ||
        performanceStarted.current ||
        !access ||
        !sortReady ||
        busy
      )
        return;
      const location = targets.find(
        (item) => item.path === performanceConfig!.root,
      );
      if (!location) return;
      performanceStarted.current = true;
      setTarget(location);
      void start(location);
    });
  });
  function performanceLayoutPending() {
    if (!performanceBuild) return;
    performanceMapReady = false;
    performanceDrawnMap = null;
  }
  function performanceRendered(nodes: NodeView[]) {
    if (performanceBuild && nodes === map) {
      performanceDrawnMap = nodes;
      performanceMapReady = true;
    }
    void finishPerformanceRender(nodes);
  }
  // The list can finish after the canvas draws. Retain the current draw identity
  // so reporting does not depend on repainting for unrelated query state.
  $effect(() => {
    const drawn = performanceDrawnMap;
    const loading = listLoading;
    untrack(() => {
      if (drawn && !loading) void finishPerformanceRender(drawn);
    });
  });
  async function finishPerformanceRender(nodes: NodeView[]) {
    if (
      !performanceBuild ||
      performanceReported.current ||
      !summary ||
      activeId.current !== summary.scanId ||
      summary.state !== "completed" ||
      summary.scanPhase !== "done" ||
      performanceMapScan !== summary.scanId ||
      nodes !== map ||
      listLoading ||
      performanceMapQueryPending
    )
      return;
    performanceReported.current = true;
    const scanId = summary.scanId;
    const canvas = canvasEvidence(nodes, focus?.id ?? summary.rootId);
    const interactions =
      performanceConfig?.interactions &&
      (!["same_process", "changed"].includes(performanceConfig.mode) ||
        performanceRescanned.current)
        ? await runNativeInteractions()
        : undefined;
    await emit("performance-rendered", {
      scanId,
      interactions,
      progressUpdates: performanceProgressUpdates,
      canvas_evidence: canvas,
    });
  }
  function start(t = target) {
    if (!t) return Promise.resolve();
    return scans.start(t);
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
    const g = how.takeView ? ++generation.current : -1;
    const performanceClock = performanceBuild
      ? emit("performance-start")
      : undefined;
    setError("");
    return {
      ready: performanceClock,
      current: () => g === generation.current,
      accepted(id) {
        // Only the newest start takes the view; the others update their sidebar rows.
        if (g !== generation.current) return;
        // A rejected target leaves the displayed workspace usable.
        homeOpen = false;
        clearResultsReturn();
        clearNavigationHistory();
        liveTarget = t;
        performanceDrawnMap = null;
        performanceMapReady = false;
        performanceProgressUpdates = 0;
        ++navigation.current;
        activeId.current = id;
        ++listRequest.current;
        setSummary(null);
        setCancelling(false);
        setTrash(null);
        setFocus(null);
        setCrumbs([]);
        setSelection([]);
        setMap([]);
        setPage({ items: [], total: 0 });
        setOffset(0);
        setQuery("");
        setDebouncedQuery("");
        setFilters({ ...emptyFilter });
      },
      event(s, restarted) {
        // Progress from a scan whose results are not on screen only updates its slot.
        if (activeId.current !== s.scanId) return;
        if (s.state === "running") performanceProgressUpdates++;
        if (s.state !== "running") setCancelling(false);
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
        setSummary(s);
      },
      fetched: setSummary,
      failed(e) {
        const accessOff = String(e).includes("FULL_DISK_ACCESS_OFF");
        // Scan all reports its failures together, but a disk swapped at its path still needs rediscovery.
        if (t.volume && String(e).includes("The chosen folder changed"))
          rediscover = true;
        if (how.batch && !accessOff) return false;
        if (!how.batch && g !== generation.current) return true;
        if (accessOff) {
          // The backend probed again and found access off: show that state. Scan all asks once.
          setAccess((old) => ({
            status: "off",
            scanAnyway: false,
            appPath: old?.appPath ?? "",
          }));
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
      key: (exploring ? displayedSlot : null) ?? (target ? slotKey(target) : null),
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
            setFocus(n);
            setCrumbs([n]);
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
    [scanIdentity, revision, selectionIdentity];
    return untrack(() => {
      if (!summary || !selection.length) return;
      const scanId = summary.scanId,
        version = revision,
        ids = selectionIdentity;
      let ignore = false;
      const current = () =>
        !ignore &&
        summary?.scanId === scanId &&
        revision === version &&
        selectionIdentity === ids;
      const nodeIds = selection.map(({ id }) => id);
      queryRefresh(() =>
        Promise.all(
          nodeIds.map((id) =>
            invoke<NodeView>("get_node", { scanId, nodeId: id }),
          ),
        )
          .then((nodes) => {
            if (!current()) return;
            const fresh = new Map(nodes.map((node) => [node.id, node]));
            setSelection((old) =>
              old.map((node) => fresh.get(node.id) ?? node),
            );
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
    [scanIdentity, mapRevision, focusIdentity, mapColorMode];
    return untrack(() => {
      if (!summary || !focus) return;
      let ignore = false;
      const mapRequest = ++performanceMapQueryEpoch;
      if (performanceBuild) performanceMapQueryPending = true;
      const snapshot = summary;
      const scanId = snapshot.scanId,
        nodeId = focus.id;
      invoke<NodeView>("get_node", { scanId, nodeId })
        .then((node) => {
          if (!ignore) setFocus(node);
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
            if (!ignore && mapRequest === performanceMapQueryEpoch) {
              // A worker can draw the old node array while the native query is
              // outstanding. Only the returned dataset can certify readiness.
              if (performanceBuild) {
                if (result.nodes !== performanceDrawnMap) {
                  performanceDrawnMap = null;
                  performanceMapReady = false;
                }
                performanceMapQueryPending = false;
              }
              setMap(result.nodes);
              if (
                performanceBuild &&
                snapshot.state === "completed" &&
                snapshot.scanPhase === "done"
              )
                setPerformanceMapScan(scanId);
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
        if (mapRequest === performanceMapQueryEpoch) {
          ++performanceMapQueryEpoch;
          if (performanceBuild) performanceMapQueryPending = false;
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
    ];
    return untrack(() => {
      const request = ++listRequest.current;
      if (!summary || !focus || !sortReady) return;
      setListLoading(true);
      // Preserve valid rows during background scan revisions; user changes
      // synchronously clear rows through invalidateList before requesting again.
      // Raw text changes invalidate prior results immediately, before debounce.
      if (query !== debouncedQuery) return;
      let ignore = false;
      const scanId = summary.scanId;
      const current = () =>
        !ignore &&
        request === listRequest.current &&
        activeId.current === scanId;
      const common = {
        scanId: summary.scanId,
        offset,
        limit: 150,
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
              ? Math.floor((result.total - 1) / 150) * 150
              : 0;
            if (offset > lastOffset) {
              // A growing directory can leave a maximum-size filter during a
              // live refresh. Request the last valid page without an inverted range.
              quickLook.dismiss();
              ++listRequest.current;
              setPage({ items: [], total: result.total });
              setOffset(lastOffset);
              return;
            }
            setPage(result);
          })
          .catch((failure) => {
            if (current()) fail(failure);
          })
          .finally(() => {
            if (current()) setListLoading(false);
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
  async function loadCrumbs(n: NodeView, scanId: string, nav: number) {
    const chain: NodeView[] = [n];
    let id = n.parentId;
    try {
      while (id !== null) {
        const parent = await invoke<NodeView>("get_node", {
          scanId,
          nodeId: id,
        });
        chain.unshift(parent);
        id = parent.parentId;
      }
      if (activeId.current === scanId && navigation.current === nav)
        setCrumbs(chain);
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
    setFocus(snapshot.focus);
    setCrumbs(snapshot.crumbs.length ? [...snapshot.crumbs] : [snapshot.focus]);
    setQuery(snapshot.query);
    setDebouncedQuery(snapshot.query);
    setFilters({ ...snapshot.filters });
    const sortChanged = sort !== snapshot.sort;
    ++sortChoice.current;
    setSort(snapshot.sort);
    setOffset(snapshot.offset);
    setSelection([...snapshot.selection]);
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
    setFocus(n);
    setOffset(0);
    setQuery("");
    setDebouncedQuery("");
    setFilters({ ...emptyFilter });
    ++listRequest.current;
    setSelection([n]);
    await loadCrumbs(n, scanId, nav);
  }
  function returnToResults() {
    const snapshot = resultsReturn;
    if (
      !snapshot ||
      !summary ||
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
  function select(n: NodeView, multi = false) {
    if (n.kind === "aggregate") return;
    if (multi || selection.length !== 1 || selection[0].id !== n.id)
      quickLook.dismiss();
    setSelection((old) =>
      multi
        ? old.some((x) => x.id === n.id)
          ? old.filter((x) => x.id !== n.id)
          : [...old, n]
        : [n],
    );
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
    setBusy(true);
    const scanId = summary.scanId;
    try {
      const review = await invoke<{ token: string; items: NodeView[] }>(
        "prepare_trash",
        { scanId, nodeIds: selection.map((n) => n.id) },
      );
      if (summary?.scanId === scanId) setTrash({ ...review, scanId });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
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
    setBusy(true);
    try {
      const result = await invoke<unknown>(
        review.queued ? "commit_cleanup_queue" : "commit_trash",
        {
          scanId,
          token: review.token,
        },
      );
      const report = cleanupResult(
        ++cleanupReportId,
        review.queued ? "queue" : "selection",
        review.items,
        result,
      );
      cleanupReport = report;
      cleanupOutcome = report.movedCount
        ? `Moved ${report.movedCount} ${report.movedCount === 1 ? "item" : "items"} to Trash.`
        : "";
      setTrash(null);
      await refreshQueue();
      if (report.movedCount || report.invalidated) {
        clearResultsReturn();
        clearNavigationHistory();
        // A cleanup that moved anything invalidates these results before a replacement scan starts.
        scans.drop(scanId);
        setSummary(null);
        await start(rescanTarget);
      }
    } catch (e) {
      setTrash(null);
      cleanupReport = {
        ...cleanupResult(
          ++cleanupReportId,
          review.queued ? "queue" : "selection",
          review.items,
          null,
        ),
        message: `Cleanup could not be confirmed. ${String(e)} Check the items in Finder before trying again.`,
      };
      cleanupOutcome = "";
      await refreshQueue();
    } finally {
      setBusy(false);
    }
  }
  const inspected = $derived(selection[selection.length - 1] || focus);
  const parentFolder = $derived(
    summary &&
      focus &&
      focus.id !== summary.rootId &&
      crumbs[0]?.id === summary.rootId
      ? crumbs.find((node) => node.id === focus.parentId)
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
    setTarget(place);
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
    quickLook.dismiss();
    clearResultsReturn();
    clearNavigationHistory();
    ++generation.current;
    activeId.current = null;
    liveTarget = null;
    setSelection([]);
    setSummary(null);
    setTrash(null);
    if (wasShown) setTarget(startup);
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
      return selectionActionable;
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
        !queueBusy &&
        !blocked &&
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
      return !accessOpen && !trash && !clearQueueOpen;
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
    get cleanupOutcome() {
      return cleanupOutcome;
    },
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
    showHome,
    showResults,
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
    get queueError() {
      return cleanupQueue.error;
    },
    cleanupQueue,
    get clearQueueOpen() {
      return clearQueueOpen;
    },
    showQueue,
    refreshQueue,
    addQueue,
    scanQueue,
    prepareQueue,
    removeQueues: cleanupQueue.remove,
    removeQueue: (entryId: string) => cleanupQueue.remove([entryId]),
    clearQueue: cleanupQueue.clear,
    setClearQueueOpen: (value: boolean) => {
      clearQueueOpen = value;
    },
    performanceLayoutPending,
    get performanceMapReady() {
      return performanceMapReady && !performanceMapQueryPending;
    },
    get performanceConfig() {
      return performanceConfig;
    },
    get performanceEventsReady() {
      return performanceEventsReady;
    },
    get performanceMapScan() {
      return performanceMapScan;
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
    get setTargets() {
      return setTargets;
    },
    get setTarget() {
      return setTarget;
    },
    get setSummary() {
      return setSummary;
    },
    get setFocus() {
      return setFocus;
    },
    get setCrumbs() {
      return setCrumbs;
    },
    get setSelection() {
      return setSelection;
    },
    get setMode() {
      return setMode;
    },
    get setOffset() {
      return setOffset;
    },
    get setError() {
      return setError;
    },
    get setBusy() {
      return setBusy;
    },
    get setCancelling() {
      return setCancelling;
    },
    get setTrash() {
      return setTrash;
    },
    get setAccessOpen() {
      return setAccessOpen;
    },
    get setAccessError() {
      return setAccessError;
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
    get generation() {
      return generation;
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
      return performanceRendered;
    },
    get start() {
      return start;
    },
    get open() {
      return open;
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
