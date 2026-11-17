import { onMount, untrack } from "svelte";
import createQuickLook from "./createQuickLook.svelte";
import { logicalSize } from "./logicalSize.svelte";
import { createCleanupQueue, tauriQueuePort } from "./cleanupQueue.svelte";
import {
  createSessionState,
  createExplorerState,
  createLibraryState,
} from "./appState.svelte";
import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import "./quickLook.css";
import {
  isSortOrder,
  type SortOrder,
  type ExplorerPreferences,
} from "./sortTypes";
import { emptyFilter, filterActive, type ExplorerFilter } from "./filterTypes";
import type { Comparison } from "./comparisonTypes";
import {
  createScanSlots,
  slotKey,
  tauriScanPort,
  type StartRequest,
  type StartView,
} from "./scanSlots.svelte";
import { latestOnly } from "./latestQuery";
import type { CleanupReport, QueueView } from "./cleanupTypes";
import { cleanupResult } from "./cleanupResults";
import type { ScanTransferReport } from "./scanTransferTypes";
import type { AppToast } from "./toastTypes";
import { canvasEvidence, runNativeInteractions } from "./nativeBenchmark";
import {
  bytes,
  size,
  type FullDiskAccess,
  type Metric,
  type SavedEntry,
  type LibraryProgress,
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
    explorer = createExplorerState(),
    library = createLibraryState();
  let performanceConfig = $state.raw<{
    root: string;
    mode: string;
    interactions?: boolean;
  } | null>(null);
  function setPerformanceConfig(
    value:
      | typeof performanceConfig
      | ((previous: typeof performanceConfig) => typeof performanceConfig),
  ) {
    performanceConfig =
      typeof value === "function" ? value(performanceConfig) : value;
  }
  const performanceStarted = { current: false };
  const performanceReported = { current: false };
  const performanceRescanned = { current: false };
  let performanceDrawnMap = $state.raw<NodeView[] | null>(null);
  let performanceProgressUpdates = 0;
  let performanceMapReady = $state.raw(false);
  let performanceMapQueryPending = $state.raw(false);
  let performanceMapQueryEpoch = 0;
  let performanceEventsReady = $state.raw(!performanceBuild);
  function setPerformanceEventsReady(
    value:
      | typeof performanceEventsReady
      | ((
          previous: typeof performanceEventsReady,
        ) => typeof performanceEventsReady),
  ) {
    performanceEventsReady =
      typeof value === "function" ? value(performanceEventsReady) : value;
  }
  let performanceMapScan = $state.raw<string | null>(null);
  function setPerformanceMapScan(
    value:
      | typeof performanceMapScan
      | ((previous: typeof performanceMapScan) => typeof performanceMapScan),
  ) {
    performanceMapScan =
      typeof value === "function" ? value(performanceMapScan) : value;
  }
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
  function changeMapColorMode(value: "folder" | "fileType") {
    mapColorMode = value;
  }
  const setSelection = explorer.setSelection;
  let selection = $derived(explorer.selection);
  const setMetric = explorer.setMetric;
  let metric = $derived(explorer.metric);
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
  // Backend master switch (SAVED_SCANS_ENABLED); off until the backend answers so the sidebar never flashes, on outside Tauri.
  let savedScansEnabled = $state.raw(!isTauri());
  let choosingFolder = $state(false);
  let folderRequest = 0;
  let disposed = false;
  // The live scans. beginStart is the app's side of every start.
  const scans = createScanSlots(tauriScanPort, { begin: beginStart });
  // Set by what holds the session besides a scan start (trash review, cleanup); starts count in scans.
  let opBusy = $state.raw(false);
  const busy = $derived(opBusy || scans.starting);
  function setBusy(
    value: typeof opBusy | ((previous: typeof opBusy) => typeof opBusy),
  ) {
    opBusy = typeof value === "function" ? value(opBusy) : value;
  }
  const setCancelling = session.setCancelling;
  let cancelling = $derived(session.cancelling);
  let trash = $state.raw<{
    queued?: boolean;
    token: string;
    items: NodeView[];
  } | null>(null);
  function setTrash(
    value: typeof trash | ((previous: typeof trash) => typeof trash),
  ) {
    trash = typeof value === "function" ? value(trash) : value;
  }
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
    mutable: () => !choosingFolder && !busy && !libraryBusy,
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
    if (busy || libraryBusy || trash || queueBusy || choosingFolder) return;
    quickLook.dismiss();
    queueOpen = value;
    if (value) {
      homeOpen = false;
      setComparisonOpen(false);
      void refreshQueue();
    }
  }
  async function addQueue() {
    if (
      !summary ||
      historical ||
      summary.state !== "completed" ||
      summary.savedId ||
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
          ...old.filter((t) => t.id !== location.id),
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
      historical ||
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
      if (summary?.scanId === scanId) setTrash({ ...review, queued: true });
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
    metric: Metric;
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
  function changeMetric(value: Metric) {
    if (value === metric || (value === "logical" && !logicalSize.shown)) return;
    invalidateList();
    setMetric(value);
    setOffset(0);
    setSelection([]);
  }
  // Hiding Logical size while viewing it returns to On disk.
  $effect(() => {
    if (!logicalSize.shown && metric === "logical")
      untrack(() => changeMetric("allocated"));
  });
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
  let access = $state.raw<FullDiskAccess | null>(null);
  function setAccess(
    value: typeof access | ((previous: typeof access) => typeof access),
  ) {
    access = typeof value === "function" ? value(access) : value;
  }
  let accessOpen = $state.raw(false);
  function setAccessOpen(
    value:
      | typeof accessOpen
      | ((previous: typeof accessOpen) => typeof accessOpen),
  ) {
    accessOpen = typeof value === "function" ? value(accessOpen) : value;
  }
  let accessBusy = $state.raw(false);
  function setAccessBusy(
    value:
      | typeof accessBusy
      | ((previous: typeof accessBusy) => typeof accessBusy),
  ) {
    accessBusy = typeof value === "function" ? value(accessBusy) : value;
  }
  let accessError = $state.raw("");
  function setAccessError(
    value:
      | typeof accessError
      | ((previous: typeof accessError) => typeof accessError),
  ) {
    accessError = typeof value === "function" ? value(accessError) : value;
  }
  // The user came back from System Settings and Full Disk Access is still off.
  let accessReturned = $state.raw(false);
  // The dialog was opened because a scan is waiting on Full Disk Access.
  let accessPrompt = $state.raw(false);
  let openedSettings = false;
  let blockedScan: {
    target: Target | null;
    savedScanId?: string;
    // The blocked request was Scan all disks: "Scan anyway" runs it again for every disk.
    scanAll?: true;
  } | null =
    null;
  const generation = { current: 0 };
  const navigation = $state({ current: 0 });
  // The Scan target of the live scan on screen, kept while a saved scan is shown.
  let liveTarget = $state.raw<Target | null>(null);
  const activeId: { current: string | null } = { current: null };
  const setSaved = library.setSaved;
  let saved = $derived(library.saved);
  const setLibraryBusy = library.setLibraryBusy;
  let libraryBusy = $derived(library.libraryBusy);
  const setLibraryProgress = library.setLibraryProgress;
  let libraryProgress = $derived(library.libraryProgress);
  const setRemoveEntry = library.setRemoveEntry;
  let removeEntry = $derived(library.removeEntry);
  const libraryLock = { current: false };
  const setComparisonOpen = library.setComparisonOpen;
  let comparisonOpen = $derived(library.comparisonOpen);
  const setComparison = library.setComparison;
  let comparison = $derived(library.comparison);
  const historical = $derived(summary?.origin === "saved");
  const setHistoricalSource = session.setHistoricalSource;
  let historicalSource = $derived(session.historicalSource);
  const blocked = $derived(busy || libraryBusy || choosingFolder);
  const running = $derived(summary?.state === "running");
  const sessionBlocked = $derived(blocked || quickLook.pending || queueBusy);
  const previewDialog = $derived(
    accessOpen || !!removeEntry || !!trash || comparisonOpen || clearQueueOpen,
  );
  $effect(() => {
    [accessOpen, removeEntry, trash, comparisonOpen, clearQueueOpen];
    return untrack(() => {
      if (previewDialog) quickLook.dismiss();
    });
  });
  function previewEligible(node: NodeView) {
    return (
      isTauri() &&
      !!summary &&
      summary.origin === "live" &&
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
  // Entries are matched against the scan of their own disk, so any slot's scan changing can change a status.
  $effect(() => {
    const identity = summary?.scanId,
      state = summary?.state,
      slotStates = scans.signature;
    untrack(() => {
      if (!performanceBuild) void refreshQueue();
    });
  });
  // The slot whose live results are on screen.
  const displayedSlot = $derived(
    summary && !historical
      ? (Object.keys(scans.slots).find(
          (k) => scans.slots[k].scanId === summary!.scanId,
        ) ?? null)
      : null,
  );
  // The scan on screen, its slot and Scan target (both null for a saved scan); null on Home or with none.
  const shown = $derived(
    summary && !homeOpen
      ? {
          summary,
          slot: displayedSlot,
          target: historical ? null : liveTarget,
        }
      : null,
  );
  // The live scan on screen, or the one last on screen while a saved scan is shown.
  const currentScan = $derived.by(() => {
    if (!historical) return summary;
    const place = liveTarget;
    return (place && scans.slots[slotKey(place)]?.summary) || null;
  });
  // A finished live scan on screen can be compared with saved scans of its location without saving it.
  const comparableCurrent = $derived.by(() => {
    const path = liveTarget?.path;
    return currentScan?.state === "completed" &&
      currentScan.origin !== "saved" &&
      path
      ? {
          id: currentScan.scanId,
          rootPath: path,
          completedAtMs: currentScan.completedAtMs ?? 0,
        }
      : null;
  });
  // The open saved scan's summary while it is not shown; the backend still holds its results.
  let parkedSaved = $state.raw<ScanSummary | null>(null);
  // A null summary is a scan that has not reported yet: an empty view that its first event fills.
  function resetView(s: ScanSummary | null, scanId = s?.scanId ?? null) {
    homeOpen = false;
    clearResultsReturn();
    clearNavigationHistory();
    quickLook.dismiss();
    setComparisonOpen(false);
    ++generation.current;
    ++navigation.current;
    activeId.current = scanId;
    setSummary(s);
    setHistoricalSource(s?.origin === "saved");
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
  function showToast(message: string, action?: AppToast["action"]) {
    toast = { id: ++toastId, message, action };
  }
  function dismissToast() {
    toast = null;
  }
  let scanTransfer = $state.raw<ScanTransferReport | null>(null);
  let scanTransferId = 0;
  let libraryOperationId = 0;
  function dismissScanTransfer() {
    if (scanTransfer?.state === "pending") return;
    scanTransfer = null;
  }
  const libraryFailures: Partial<
    Record<LibraryProgress["operation"], { title: string; reason: string }>
  > = {
    save: {
      title: "Couldn’t save the scan",
      reason:
        "Hahakigami could not save this scan to Saved Scans. Check that your disk has room, then try again.",
    },
    compare: {
      title: "Couldn’t compare the scans",
      reason:
        "Hahakigami could not read one of the saved scans. The file may be damaged. Try again, or pick other scans.",
    },
  };
  async function libraryOperation<T>(
    operation: LibraryProgress["operation"],
    action: (channel: Channel<LibraryProgress>) => Promise<T>,
    hooks?: {
      onProgress?: (progress: LibraryProgress) => void;
      onError?: (error: unknown) => void | Promise<void>;
    },
  ): Promise<T | undefined> {
    if (
      disposed ||
      libraryLock.current ||
      choosingFolder ||
      quickLook.isPending()
    )
      return;
    const request = ++libraryOperationId;
    let acceptsProgress = true;
    const current = () =>
      !disposed && libraryLock.current && request === libraryOperationId;
    libraryLock.current = true;
    setLibraryBusy(true);
    setError("");
    setLibraryProgress({
      operation,
      phase:
        operation === "import" || operation === "export"
          ? "choosing"
          : operation === "save"
            ? "writing"
            : operation === "compare"
              ? "readingBefore"
              : "reading",
      bytes: 0,
    });
    const channel = new Channel<LibraryProgress>();
    channel.onmessage = (p) => {
      if (
        !acceptsProgress ||
        !current() ||
        !p ||
        p.operation !== operation ||
        !Number.isSafeInteger(p.bytes) ||
        p.bytes < 0
      )
        return;
      const phases: Record<LibraryProgress["operation"], readonly string[]> = {
        import: ["choosing", "reading", "writing", "validating", "done"],
        export: ["choosing", "reading", "writing", "validating", "done"],
        open: ["reading", "validating", "done"],
        save: ["writing", "validating", "done"],
        compare: ["readingBefore", "readingAfter", "comparing", "done"],
      };
      if (!phases[operation].includes(p.phase)) return;
      setLibraryProgress(p);
      hooks?.onProgress?.(p);
    };
    try {
      const value = await action(channel);
      return current() ? value : undefined;
    } catch (e) {
      acceptsProgress = false;
      channel.onmessage = () => {};
      if (current()) {
        if (hooks?.onError) await hooks.onError(e);
        else setError(String(e), libraryFailures[operation] ?? null);
      }
    } finally {
      acceptsProgress = false;
      libraryLock.current = false;
      if (!disposed) {
        setLibraryBusy(false);
        setLibraryProgress(null);
      }
      channel.onmessage = () => {};
    }
  }
  let openingSavedEntry = $state.raw<SavedEntry | null>(null);
  let savedOpenCancelling = $state(false);
  let savedOpenRequest: string | null = null;
  async function cancelSavedOpen() {
    if (libraryProgress?.operation !== "open" || savedOpenCancelling) return;
    savedOpenCancelling = true;
    if (savedOpenRequest) {
      try {
        await invoke<boolean>("cancel_saved_open", {
          requestId: savedOpenRequest,
        });
      } catch (error) {
        savedOpenCancelling = false;
        setError(String(error), {
          title: "Couldn’t cancel opening",
          reason:
            "Hahakigami could not stop opening this saved scan. Wait for it to finish.",
        });
      }
    }
  }
  async function openSaved(id: string) {
    if (
      sessionBlocked ||
      quickLook.isPending() ||
      running ||
      scans.anyRunning ||
      libraryLock.current
    )
      return;
    if (parkedSaved?.savedId === id) {
      resetView(parkedSaved);
      parkedSaved = null;
      closeAccess();
      return;
    }
    openingSavedEntry = saved.find((entry) => entry.id === id) ?? null;
    try {
      const s = await libraryOperation(
        "open",
        async (onProgress) => {
          savedOpenRequest = await invoke<string>("begin_saved_open");
          if (savedOpenCancelling) {
            try {
              await invoke<boolean>("cancel_saved_open", {
                requestId: savedOpenRequest,
              });
            } catch (error) {
              savedOpenCancelling = false;
              setError(String(error), {
                title: "Couldn’t cancel opening",
                reason:
                  "Hahakigami could not stop opening this saved scan. Wait for it to finish.",
              });
            }
          }
          return invoke<ScanSummary>("open_saved_scan", {
            id,
            requestId: savedOpenRequest,
            onProgress,
          });
        },
        {
          onError: (error) => {
            if (String(error) !== "Saved scan opening cancelled.")
              setError(String(error), {
                title: "Couldn’t open the saved scan",
                reason:
                  "Hahakigami could not open this saved scan. The file may be damaged. You can remove it from Saved Scans.",
              });
          },
        },
      );
      if (s) {
        resetView(s);
        parkedSaved = null;
        closeAccess();
      }
    } finally {
      openingSavedEntry = null;
      savedOpenRequest = null;
      savedOpenCancelling = false;
    }
  }
  async function saveScan() {
    if (
      !savedScansEnabled ||
      !summary ||
      historical ||
      summary.state !== "completed" ||
      sessionBlocked ||
      quickLook.isPending() ||
      libraryLock.current
    )
      return;
    const scanId = summary.scanId;
    const entry = await libraryOperation("save", (onProgress) =>
      invoke<SavedEntry>("save_scan", { scanId, onProgress }),
    );
    if (!entry) return;
    // Link the live scan to its saved entry so Save can't create a duplicate.
    setSummary((old) =>
      old?.scanId === scanId ? { ...old, savedId: entry.id } : old,
    );
    scans.patch(scanId, { savedId: entry.id });
    setSaved((old) =>
      [entry, ...old].sort((a, b) => b.savedAtMs - a.savedAtMs),
    );
    showToast("Scan saved.");
  }
  async function compareScans(beforeId: string, afterId: string) {
    if (
      sessionBlocked ||
      quickLook.isPending() ||
      running ||
      scans.anyRunning ||
      libraryLock.current
    )
      return;
    const current = comparableCurrent;
    const currentAfter = afterId === current?.id;
    const result = await libraryOperation("compare", (onProgress) =>
      current && (currentAfter || beforeId === current.id)
        ? invoke<Comparison>("compare_current_scan", {
            scanId: current.id,
            savedId: currentAfter ? beforeId : afterId,
            currentAfter,
            onProgress,
          })
        : invoke<Comparison>("compare_saved_scans", {
            beforeId,
            afterId,
            onProgress,
          }),
    );
    if (result) setComparison(result);
  }
  function isSavedEntry(value: unknown): value is SavedEntry {
    if (!value || typeof value !== "object") return false;
    const entry = value as Record<string, unknown>;
    return (
      typeof entry.id === "string" &&
      !!entry.id &&
      typeof entry.name === "string" &&
      typeof entry.rootPath === "string" &&
      typeof entry.incomplete === "boolean" &&
      ["startedAtMs", "completedAtMs", "savedAtMs"].every(
        (key) =>
          typeof entry[key] === "number" &&
          Number.isFinite(entry[key]) &&
          !Number.isNaN(new Date(entry[key] as number).getTime()),
      ) &&
      ["logicalBytes", "allocatedBytes"].every(
        (key) => Number.isInteger(entry[key]) && (entry[key] as number) >= 0,
      ) &&
      ["entries", "errors", "excluded", "fileBytes"].every(
        (key) =>
          Number.isSafeInteger(entry[key]) && (entry[key] as number) >= 0,
      )
    );
  }
  async function transferScan(
    operation: ScanTransferReport["operation"],
    exportId?: string,
  ) {
    if (
      disposed ||
      !isTauri() ||
      sessionBlocked ||
      quickLook.isPending() ||
      running ||
      scans.anyRunning ||
      libraryLock.current
    )
      return;
    quickLook.dismiss();
    const entry =
      operation === "export"
        ? saved.find((entry) => entry.id === exportId)
        : null;
    const id = ++scanTransferId;
    const current = () => !disposed && scanTransfer?.id === id;
    const requestCurrent = () =>
      !disposed && scanTransferId === id && libraryLock.current;
    await libraryOperation(
      operation,
      async (onProgress) => {
        scanTransfer = {
          id,
          operation,
          state: "pending",
          phase: "choosing",
          bytes: 0,
          entry: entry ? { ...entry } : null,
          error: null,
        };
        const value: unknown = await (operation === "import"
          ? invoke("import_saved_scan", { onProgress })
          : invoke("export_saved_scan", { id: exportId, onProgress }));
        if (!current()) return;
        const cancelled =
          operation === "import" ? value === null : value === false;
        if (
          !cancelled &&
          (operation === "import" ? !isSavedEntry(value) : value !== true)
        )
          throw new Error("The transfer response could not be confirmed.");
        const retry = {
          label:
            operation === "import"
              ? "Choose file again"
              : "Choose destination again",
          run: () => void transferScan(operation, exportId),
        };
        if (operation === "import" && !cancelled && isSavedEntry(value)) {
          const imported = { ...value };
          setSaved((old) =>
            [imported, ...old.filter((entry) => entry.id !== imported.id)].sort(
              (a, b) => b.savedAtMs - a.savedAtMs,
            ),
          );
          scanTransfer = null;
          showToast("Imported scan added to Saved Scans.", {
            label: "Open imported scan",
            run: () => void openSaved(imported.id),
          });
        } else {
          scanTransfer = null;
          if (cancelled)
            showToast(
              `${operation === "import" ? "Import" : "Export"} cancelled. Your current results remain available.`,
              retry,
            );
          else {
            showToast("Scan exported. You can share the .hahakigamiscan file.");
          }
        }
      },
      {
        onProgress: (progress) => {
          if (!current() || scanTransfer?.state !== "pending") return;
          scanTransfer = {
            ...scanTransfer,
            phase:
              progress.phase === "choosing"
                ? "choosing"
                : progress.phase === "done"
                  ? "finishing"
                  : "processing",
            bytes: progress.bytes,
          };
        },
        onError: async (error) => {
          if (!current()) return;
          scanTransfer = {
            ...scanTransfer!,
            state: "failed",
            error: String(error),
          };
          // Publication may have succeeded before an import's final metadata write failed.
          // Refresh the library under the same lock without restoring lastOpenedId.
          if (operation === "import") {
            try {
              const list = await invoke<{ entries: unknown[] }>(
                "list_saved_scans",
              );
              if (
                requestCurrent() &&
                Array.isArray(list?.entries) &&
                list.entries.every(isSavedEntry)
              )
                setSaved(
                  [...list.entries].sort((a, b) => b.savedAtMs - a.savedAtMs),
                );
            } catch {
              // Keep the original unconfirmed transfer error and existing library view.
            }
          }
        },
      },
    );
  }
  async function importScan() {
    await transferScan("import");
  }
  async function exportScan(id: string) {
    await transferScan("export", id);
  }
  async function removeSaved() {
    if (
      !removeEntry ||
      sessionBlocked ||
      quickLook.isPending() ||
      libraryLock.current
    )
      return;
    libraryLock.current = true;
    setLibraryBusy(true);
    setError("");
    try {
      await invoke("delete_saved_scan", { id: removeEntry.id });
      // The backend closed the deleted saved scan; leave it for the live scan behind it or Home.
      if (historical && summary?.savedId === removeEntry.id) {
        if (currentScan && liveTarget)
          showSlot(slotKey(liveTarget));
        else {
          quickLook.dismiss();
          clearResultsReturn();
          clearNavigationHistory();
          ++generation.current;
          activeId.current = null;
          setHistoricalSource(false);
          setSelection([]);
          setSummary(null);
          showHome();
        }
      }
      setSaved((old) => old.filter((e) => e.id !== removeEntry.id));
      setSummary((old) =>
        old?.savedId === removeEntry.id ? { ...old, savedId: null } : old,
      );
      for (const slot of Object.values(scans.slots))
        if (slot.summary?.savedId === removeEntry.id)
          scans.patch(slot.scanId, { savedId: null });
      if (parkedSaved?.savedId === removeEntry.id) parkedSaved = null;
      setRemoveEntry(null);
    } catch (e) {
      setError(String(e), {
        title: "Couldn’t remove the saved scan",
        reason:
          "Hahakigami could not delete this saved scan. Check the Saved Scans list, then try again.",
      });
    } finally {
      libraryLock.current = false;
      setLibraryBusy(false);
    }
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
    else if (request) await start(request.target, request.savedScanId);
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
      libraryLock.current ||
      quickLook.isPending() ||
      accessOpen ||
      !!removeEntry ||
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
      previousSummary = summary,
      previousComparisonOpen = comparisonOpen;
    const current = () =>
      !disposed &&
      request === folderRequest &&
      generation.current === g &&
      navigation.current === nav &&
      activeId.current === scanId &&
      target === previousTarget &&
      // Progress replaces the summary object of a scan that keeps running while the dialog is open.
      summary?.scanId === previousSummary?.scanId &&
      comparisonOpen === previousComparisonOpen;
    quickLook.dismiss();
    try {
      const chosen = await invoke<Target | null>("choose_folder");
      if (!current()) return;
      if (chosen) {
        setComparisonOpen(false);
        setTarget(chosen);
        setHistoricalSource(false);
        setTargets((previous) =>
          previous.some((item) => item.id === chosen.id)
            ? previous
            : [...previous, chosen],
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
  function fail(e: unknown, context?: { title: string; reason: string }) {
    setError(String(e), context);
  }
  onMount(() => {
    let cancelled = false;
    if (isTauri()) {
      void loadAccess(() => !cancelled);
      setLibraryBusy(true);
      libraryLock.current = true;
      const loadSaved = () =>
        invoke<{
          entries: SavedEntry[];
          lastOpenedId: string | null;
        }>("list_saved_scans")
          .then(async (list) => {
            if (cancelled) return;
            setSaved(list.entries.sort((a, b) => b.savedAtMs - a.savedAtMs));
            if (list.lastOpenedId) {
              libraryLock.current = false;
              setLibraryBusy(false);
              await openSaved(list.lastOpenedId);
            } else {
              libraryLock.current = false;
              setLibraryBusy(false);
            }
          })
          .catch((e) => {
            if (cancelled) return;
            setError(String(e), {
              title: "Couldn’t load Saved Scans",
              reason:
                "Hahakigami could not read your list of Saved Scans. Restart Hahakigami to try again.",
            });
            libraryLock.current = false;
            setLibraryBusy(false);
          });
      // Ask the backend first; with saved scans off, never list or reopen the library.
      // A failed probe keeps the library off rather than calling commands that may refuse.
      const unlockLibrary = () => {
        libraryLock.current = false;
        setLibraryBusy(false);
      };
      invoke<boolean>("saved_scans_enabled").then((on) => {
        savedScansEnabled = on !== false;
        if (savedScansEnabled) return loadSaved();
        unlockLibrary();
      }, unlockLibrary);
    }
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
          (mounted.length === shown.length &&
            shown.every((item) => mounted.includes(item.path)))
        )
          return;
        const found = (await invoke<Target[]>("discover_targets")).filter(
          (item) => item.volume,
        );
        // A scan starting now owns the view; the next tick retries.
        if (cancelled || busy) return;
        const present = new Set(found.map((item) => item.path));
        const kept = targets.filter(
          (item) => !item.volume || present.has(item.path),
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
        setTargets(next);
        dropEjected(gone, startup);
        if (target?.volume && !kept.includes(target)) setTarget(startup);
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
      libraryBusy,
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
        libraryBusy ||
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
    const canvas = canvasEvidence(nodes, focus?.id ?? summary.rootId, metric);
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
  function start(t = target, savedScanId?: string) {
    if (!t && !savedScanId) return Promise.resolve();
    return scans.start(savedScanId ? { savedScanId } : { target: t! });
  }
  // The app's side of a start, for the scan slots: its gates, then what the view does as the scan
  // reports. Only a start that no newer start or navigation overtook (`g`) takes the view.
  function beginStart(
    request: StartRequest,
    how: { takeView: boolean; batch: boolean },
  ): StartView | null {
    const t = "target" in request ? request.target : null;
    const savedScanId =
      "savedScanId" in request ? request.savedScanId : undefined;
    const blocking = {
      target: t,
      savedScanId,
      scanAll: how.batch ? (true as const) : undefined,
    };
    if (
      choosingFolder ||
      libraryLock.current ||
      quickLook.isPending() ||
      scanBlocked(blocking)
    )
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
        if (historical) parkedSaved = summary;
        setHistoricalSource(false);
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
          showToast("Scan cancelled. Partial results are shown.");
        setSummary(s);
      },
      target(result) {
        setTargets((old) => [
          ...old.filter((existing) => existing.id !== result.id),
          result,
        ]);
        if (g === generation.current) {
          liveTarget = result;
          setTarget(result);
        }
      },
      fetched: setSummary,
      failed(e) {
        const accessOff = String(e).includes("FULL_DISK_ACCESS_OFF");
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
        } else fail(e);
        return true;
      },
    };
  }
  // Scan all disks covers the local disks only: a network share is slow and can stall.
  const localDisks = $derived(
    targets.filter((item) => item.volume && item.local === true),
  );
  // Starts a fresh scan of every local disk; one that is scanning is cancelled first. A refused start
  // does not stop the others: the failures are reported together.
  async function scanAllDisks() {
    if (!can.scanAllDisks) return;
    if (scanBlocked({ target: localDisks[0], scanAll: true })) return;
    // Only the start of the disk on screen (else the selected one) takes the view, and only if the user has not selected or opened
    // anything else since the press; the other starts just update their sidebar rows.
    const pressed = {
      generation: generation.current,
      id: target?.id,
      home: homeOpen,
      // The disk whose results are on screen, else the selected one.
      key: (exploring ? displayedSlot : null) ?? (target ? slotKey(target) : null),
    };
    const failures = await scans.scanAll(() => localDisks, {
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
          if (!ignore) {
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
      if (!summary || historical || !selection.length) return;
      const scanId = summary.scanId,
        version = revision,
        ids = selectionIdentity;
      let ignore = false;
      const current = () =>
        !ignore &&
        summary?.scanId === scanId &&
        revision === version &&
        selectionIdentity === ids;
      void Promise.all(
        selection.map(({ id }) =>
          invoke<NodeView>("get_node", { scanId, nodeId: id }),
        ),
      )
        .then((nodes) => {
          if (!current()) return;
          const fresh = new Map(nodes.map((node) => [node.id, node]));
          setSelection((old) => old.map((node) => fresh.get(node.id) ?? node));
        })
        .catch((failure) => {
          if (current()) fail(failure);
        });
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
  $effect(() => {
    [scanIdentity, mapRevision, focusIdentity, metric, mapColorMode];
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
          metric,
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
      else void loadMap();
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
      revision,
      focusIdentity,
      metric,
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
        metric,
        sort,
      };
      const nameQuery = debouncedQuery.trim();
      const load = filtered
        ? invoke<Page>("filter_nodes", {
            ...common,
            query: nameQuery,
            ...filters,
          })
        : nameQuery
          ? invoke<Page>("search_nodes", {
              ...common,
              nodeId: focus.id,
              query: nameQuery,
            })
          : invoke<Page>("get_children", { ...common, nodeId: focus.id });
      load
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
      metric,
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
    saved: ResultsReturn,
    returnPoint: ResultsReturn | null,
  ) {
    const nav = ++navigation.current;
    invalidateList();
    setFocus(saved.focus);
    setCrumbs(saved.crumbs.length ? [...saved.crumbs] : [saved.focus]);
    setQuery(saved.query);
    setDebouncedQuery(saved.query);
    setFilters({ ...saved.filters });
    setMetric(saved.metric);
    const sortChanged = sort !== saved.sort;
    ++sortChoice.current;
    setSort(saved.sort);
    setOffset(saved.offset);
    setSelection([...saved.selection]);
    listScroll = saved.scroll;
    listScrollRestore = { id: ++scrollRestoreId, top: saved.scroll };
    resultsReturn = returnPoint;
    if (sortChanged) void persistSort(saved.sort);
    if (!saved.crumbs.length) await loadCrumbs(saved.focus, saved.scanId, nav);
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
      !queueOpen &&
      !comparisonOpen
    );
  }
  async function goBack() {
    if (!canNavigateHistory(backHistory)) return;
    const current = captureView()!;
    const saved = backHistory.at(-1)!;
    backHistory = backHistory.slice(0, -1);
    forwardHistory = [...forwardHistory.slice(-99), current];
    await restoreView(saved, saved.resultsReturn);
  }
  async function goForward() {
    if (!canNavigateHistory(forwardHistory)) return;
    const current = captureView()!;
    const saved = forwardHistory.at(-1)!;
    forwardHistory = forwardHistory.slice(0, -1);
    backHistory = [...backHistory.slice(-99), current];
    await restoreView(saved, saved.resultsReturn);
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
    const saved = resultsReturn;
    if (
      !saved ||
      !summary ||
      saved.scanId !== summary.scanId ||
      activeId.current !== saved.scanId ||
      sessionBlocked ||
      previewDialog
    )
      return;
    const current = captureView();
    if (current) rememberView(current);
    void restoreView(saved, null);
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
      historical ||
      summary.state !== "completed" ||
      !selection.length ||
      sessionBlocked ||
      quickLook.isPending() ||
      selection.some((node) => node.excluded)
    )
      return;
    quickLook.dismiss();
    setBusy(true);
    try {
      setTrash(
        await invoke("prepare_trash", {
          scanId: summary.scanId,
          nodeIds: selection.map((n) => n.id),
        }),
      );
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }
  async function commitTrash() {
    if (!summary || historical || libraryBusy || busy || !trash) return;
    const review = {
      ...trash,
      items: trash.items.map((item) => ({ ...item })),
    };
    const scanId = summary.scanId;
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
      if (report.movedCount) {
        clearResultsReturn();
        clearNavigationHistory();
        // Successful cleanup invalidates these results before a replacement scan starts.
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
  const restrictedIssues = $derived(
    Math.min(
      summary?.errors ?? 0,
      Math.max(0, summary?.restrictedSystemIssues ?? 0),
    ),
  );
  const otherIssues = $derived(
    Math.max(0, (summary?.errors ?? 0) - restrictedIssues),
  );
  const inspected = $derived(selection[selection.length - 1] || focus);
  const parentFolder = $derived(
    summary &&
      focus &&
      focus.id !== summary.rootId &&
      crumbs[0]?.id === summary.rootId
      ? crumbs.find((node) => node.id === focus.parentId)
      : undefined,
  );
  const selectedSavedEntry = $derived(
    saved.find((entry) => entry.id === summary?.savedId),
  );
  // A selected target describes the next scan, never the identity of existing results.
  const resultLocation = $derived.by(() => {
    if (!summary) {
      return openingSavedEntry
        ? { name: openingSavedEntry.name, path: openingSavedEntry.rootPath }
        : null;
    }
    const root = crumbs[0];
    const recorded = historical ? selectedSavedEntry : null;
    const scanned = historical ? null : liveTarget;
    const path = scanned?.path ?? root?.path ?? recorded?.rootPath;
    return path
      ? { name: scanned?.name ?? root?.name ?? recorded?.name, path }
      : null;
  });
  const displayedTargetSelected = $derived(
    !!summary &&
      !!resultLocation &&
      ((historical && historicalSource) ||
        target?.path === resultLocation.path),
  );
  // Scan the location of the results on screen, whatever the sidebar has selected.
  function rescanDisplayed() {
    return historical
      ? start(null, summary?.scanId)
      : start(liveTarget);
  }
  function rescan() {
    return displayedTargetSelected && !homeOpen
      ? rescanDisplayed()
      : start(target);
  }
  function showHome() {
    quickLook.dismiss();
    queueOpen = false;
    setComparisonOpen(false);
    homeOpen = true;
  }
  // Leave Home, the queue or the comparison for the results already loaded.
  function showResults() {
    quickLook.dismiss();
    queueOpen = false;
    setComparisonOpen(false);
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
      const open = historical ? summary : null;
      liveTarget = slot.target;
      resetView(slot.summary, slot.scanId);
      if (open) parkedSaved = open;
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
    setHistoricalSource(false);
    setSelection([]);
    setSummary(null);
    if (wasShown) setTarget(startup);
    void refreshQueue();
  }
  function openComparison() {
    if (!savedScansEnabled) return;
    showQueue(false);
    homeOpen = false;
    quickLook.dismiss();
    setComparisonOpen(true);
    closeAccess();
    setTrash(null);
  }
  // The explorer (map, file list, inspector) is what the user is looking at.
  const exploring = $derived(
    !!summary && !homeOpen && !queueOpen && !comparisonOpen,
  );
  const selectionActionable = $derived(
    exploring &&
      !historical &&
      !libraryBusy &&
      summary?.state === "completed" &&
      selection.length > 0 &&
      !busy &&
      !quickLook.pending &&
      !selection.some((node) => node.excluded),
  );
  // For what cannot overlap any scan on any disk (the saved-scan library); backend gates match.
  // Starting a scan only needs its own slot free: see can.rescan.
  const idle = $derived(!running && !scans.anyRunning && !sessionBlocked);
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
        !queueOpen &&
        !comparisonOpen
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
        !queueBusy &&
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
        !historical &&
        shown?.summary.state === "completed" &&
        readyHereEntries.length > 0
      );
    },
    get reveal() {
      return (
        exploring &&
        !historical &&
        !libraryBusy &&
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
      // What the click scans: the displayed results' place (a saved scan rescans in the folder slot)
      // or the sidebar target. Only that slot has to be free; other disks may be scanning.
      const savedRescan = onDisplayed && historical;
      const place = onDisplayed ? liveTarget : target;
      return (
        (savedRescan || !!place) &&
        !sessionBlocked &&
        !(savedRescan ? scans.folderBusy() : scans.busy(place!)) &&
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
        localDisks.length > 0
      );
    },
    // Dialogs about the results on screen: they rescan those, not the sidebar target.
    get rescanDisplayed() {
      return (
        !!summary &&
        !homeOpen &&
        (historical || !!liveTarget) &&
        !sessionBlocked &&
        !(historical ? scans.folderBusy() : scans.busy(liveTarget!)) &&
        !!access
      );
    },
    get stopScan() {
      return (
        !!summary &&
        running &&
        !historical &&
        !cancelling &&
        summary.scanPhase !== "caching"
      );
    },
    get saveScan() {
      return (
        savedScansEnabled &&
        summary?.state === "completed" &&
        !historical &&
        !sessionBlocked &&
        !summary.savedId
      );
    },
    get chooseFolder() {
      return isTauri() && !sessionBlocked && !targetsLoading && !!access;
    },
    get importScan() {
      return savedScansEnabled && isTauri() && idle;
    },
    /** Any saved scan's own Export action. */
    get exportSaved() {
      return savedScansEnabled && isTauri() && idle;
    },
    /** Export the displayed saved scan. */
    get exportScan() {
      return savedScansEnabled && isTauri() && idle && !!selectedSavedEntry;
    },
    get compareScans() {
      return (
        savedScansEnabled &&
        idle &&
        (saved.length >= 2 ||
          saved.some((entry) => entry.rootPath === comparableCurrent?.rootPath))
      );
    },
    get cleanupQueue() {
      return !blocked && !trash;
    },
    get find() {
      return exploring;
    },
    get explore() {
      return exploring;
    },
    get logical() {
      return exploring && logicalSize.shown;
    },
    get toggleSidebar() {
      return !accessOpen && !removeEntry && !trash && !clearQueueOpen;
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
    get selectedSavedEntry() {
      return selectedSavedEntry;
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
    openComparison,
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
    showSlot,
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
    get comparableCurrent() {
      return comparableCurrent;
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
    get metric() {
      return metric;
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
    get saved() {
      return saved;
    },
    get libraryBusy() {
      return libraryBusy;
    },
    get libraryProgress() {
      return libraryProgress;
    },
    get savedOpenCancelling() {
      return savedOpenCancelling;
    },
    get cancelSavedOpen() {
      return cancelSavedOpen;
    },
    get openingSavedEntry() {
      return openingSavedEntry;
    },
    get removeEntry() {
      return removeEntry;
    },
    get comparisonOpen() {
      return comparisonOpen;
    },
    get comparison() {
      return comparison;
    },
    get historicalSource() {
      return historicalSource;
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
    get setPage() {
      return setPage;
    },
    get setMap() {
      return setMap;
    },
    get setSelection() {
      return setSelection;
    },
    get setMetric() {
      return setMetric;
    },
    get setMode() {
      return setMode;
    },
    get setQuery() {
      return setQuery;
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
    get setFilters() {
      return setFilters;
    },
    get setDebouncedQuery() {
      return setDebouncedQuery;
    },
    get setListLoading() {
      return setListLoading;
    },
    get setSort() {
      return setSort;
    },
    get setSortReady() {
      return setSortReady;
    },
    get setSortSaving() {
      return setSortSaving;
    },
    get setSortWarning() {
      return setSortWarning;
    },
    get setAccess() {
      return setAccess;
    },
    get setAccessOpen() {
      return setAccessOpen;
    },
    get setAccessBusy() {
      return setAccessBusy;
    },
    get setAccessError() {
      return setAccessError;
    },
    get setSaved() {
      return setSaved;
    },
    get setLibraryBusy() {
      return setLibraryBusy;
    },
    get setLibraryProgress() {
      return setLibraryProgress;
    },
    get setRemoveEntry() {
      return setRemoveEntry;
    },
    get setComparisonOpen() {
      return setComparisonOpen;
    },
    get setComparison() {
      return setComparison;
    },
    get setHistoricalSource() {
      return setHistoricalSource;
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
    get historical() {
      return historical;
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
    get revision() {
      return revision;
    },
    get restrictedIssues() {
      return restrictedIssues;
    },
    get otherIssues() {
      return otherIssues;
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
    get libraryLock() {
      return libraryLock;
    },
    get invalidateList() {
      return invalidateList;
    },
    get changeFilters() {
      return changeFilters;
    },
    get changeMetric() {
      return changeMetric;
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
    get libraryOperation() {
      return libraryOperation;
    },
    get openSaved() {
      return openSaved;
    },
    get importScan() {
      return importScan;
    },
    get exportScan() {
      return exportScan;
    },
    get scanTransfer() {
      return scanTransfer;
    },
    dismissScanTransfer,
    get toast() {
      return toast;
    },
    dismissToast,
    get saveScan() {
      return saveScan;
    },
    get savedScansEnabled() {
      return savedScansEnabled;
    },
    get liveScanSaved() {
      return !historical && !!summary?.savedId;
    },
    get compareScans() {
      return compareScans;
    },
    get removeSaved() {
      return removeSaved;
    },
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
