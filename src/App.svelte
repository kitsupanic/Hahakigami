<script lang="ts">
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { flushSync, onMount, untrack } from "svelte";
  import type { HoverTarget } from "./hoverContext";
  import type { QueueEntry } from "./cleanupTypes";
  import {
    bytes,
    PAGE_SIZE,
    type NodeView,
    type Page,
    type Target,
  } from "./types";
  import { SvelteSet } from "svelte/reactivity";
  import { compactBytes } from "./uiFormat";
  import type { FolderRow } from "./queueFolders";
  import createApp from "./createApp.svelte";
  import {
    createAppMenu,
    isTextField,
    type AppMenu,
    type MenuCommands,
    type MenuState,
  } from "./appMenu";
  import { applyAppearance, applyGuardian, theme } from "./theme.svelte";
  import {
    dialogFocus,
    dismissPopover,
    captureDialogOpener,
  } from "./dialogFocus";
  let dialogOpener = $state.raw<HTMLElement | null>(null);
  let trashSelectedCount = $state<number | undefined>();
  // The scans whose failure the user already closed, so they do not reopen.
  const dismissedFailedScans = new SvelteSet<string>();
  const listViews = [
    {
      name: "files",
      id: "files-tab",
      panel: "files-panel",
      label: "Folders and files",
    },
    {
      name: "fileTypes",
      id: "file-types-tab",
      panel: "file-types-panel",
      label: "File types",
    },
  ] as const;
  let listView = $state<"files" | "fileTypes">("files");
  let locationsSidebarVisible = $state(true);
  let sidebarWidth = $state<number | null>(null);
  let listVisible = $state(true);
  let fileTypesMounted = $state(false);
  let filtersOpen = $state(false);
  // Sidebars slide instead of snapping: a view transition morphs the panels between the
  // two layouts. Without the API (or with Reduce Motion) the change is instant.
  function animateLayout(change: () => void) {
    const still = window.matchMedia?.(
      "(prefers-reduced-motion: reduce)",
    ).matches;
    if (still || !document.startViewTransition) return change();
    document.startViewTransition(() => flushSync(change));
  }
  function toggleSidebar() {
    animateLayout(() => (locationsSidebarVisible = !locationsSidebarVisible));
  }
  function toggleFileList() {
    animateLayout(() => (listVisible = !listVisible));
    filtersOpen = false;
  }
  let filtersStyle = $state("");
  function toggleFilters(event: MouseEvent) {
    if (filtersOpen) {
      filtersOpen = false;
      return;
    }
    const button = event.currentTarget as HTMLButtonElement;
    const rect = button.getBoundingClientRect();
    // Stay inside the list column (or the viewport when it is not laid out yet).
    const column = button.closest(".workspace-side")?.getBoundingClientRect();
    const mainLeft = button.closest("main")?.getBoundingClientRect().left ?? 0;
    const minLeft = Math.max(mainLeft, column?.width ? column.left : 0) + 12;
    const maxRight =
      Math.min(window.innerWidth, column?.width ? column.right : Infinity) - 12;
    const width = Math.min(490, maxRight - minLeft);
    const height = Math.min(480, window.innerHeight - 24);
    // Right-align to the trigger, then clamp.
    const left = Math.max(
      minLeft,
      Math.min(rect.right - width, maxRight - width),
    );
    const top = Math.max(
      12,
      Math.min(rect.bottom + 6, window.innerHeight - height - 12),
    );
    filtersStyle = `top: ${top}px; left: ${left}px; width: ${width}px; max-height: ${height}px`;
    filtersOpen = true;
  }
  let listScrollbarWidth = $state(0);
  // The list column leads the map, so the divider tracks the list's share.
  let mapShare = $state(DEFAULT_MAP_SHARE);
  import { DEFAULT_MAP_SHARE } from "./explorerPresentation";
  import Sidebar from "./Sidebar.svelte";
  import Icon from "./Icon.svelte";
  import appIcon from "../src-tauri/icons/128x128.png";
  import { createBranchColorResolver } from "./branchColors";
  import DiskMap from "./DiskMap.svelte";
  import FileRows from "./FileRows.svelte";
  import ScanStatusLine from "./ScanStatusLine.svelte";
  import TrashConfirmation from "./TrashConfirmation.svelte";
  import CleanupResults from "./CleanupResults.svelte";
  import Modal from "./Modal.svelte";
  import FailureModal from "./FailureModal.svelte";
  import Toast from "./Toast.svelte";
  import ExplorerFilters from "./ExplorerFilters.svelte";
  import ActiveFilters from "./ActiveFilters.svelte";
  import SortControl from "./SortControl.svelte";
  import { sortLabels } from "./sortTypes";
  import PanelDivider from "./PanelDivider.svelte";
  import SidebarDivider from "./SidebarDivider.svelte";
  import FileTypes from "./FileTypes.svelte";
  import createFileTypes from "./createFileTypes.svelte";
  import { emptyFilter } from "./filterTypes";
  const macosTitlebar =
    isTauri() &&
    typeof navigator !== "undefined" &&
    navigator.platform.startsWith("Mac");
  const app = createApp();
  const mapColorMode = $derived(app.mapColorMode);
  const fileTypes = createFileTypes({
    get summary() {
      return app.summary;
    },
    get focus() {
      return app.focus;
    },
    get nodes() {
      return app.map;
    },
    get enabled() {
      return listView === "fileTypes";
    },
  });
  function showListView(view: "files" | "fileTypes") {
    if (view === "fileTypes") fileTypesMounted = true;
    listView = view;
  }
  // The inspector tab is the one control for what the map colours by.
  $effect(() => {
    const mode = listView === "fileTypes" ? "fileType" : "folder";
    untrack(() => app.changeMapColorMode(mode));
  });
  function listViewKey(event: KeyboardEvent) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const tabs = [
      ...(
        event.currentTarget as HTMLButtonElement
      ).parentElement!.querySelectorAll<HTMLButtonElement>("[role=tab]"),
    ];
    const current = listViews.findIndex((view) => view.name === listView);
    const index =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? listViews.length - 1
          : (current +
              (event.key === "ArrowRight" ? 1 : -1) +
              listViews.length) %
            listViews.length;
    showListView(listViews[index].name);
    tabs[index]?.focus();
  }
  function showFileTypeFiles() {
    const extension = fileTypes.extension;
    if (extension === null || fileTypes.nodeId === undefined) return;
    app.changeQuery("");
    app.changeFilters({ ...emptyFilter, extension, scopeId: fileTypes.nodeId });
    mapShare = 0.32;
    listVisible = true;
    showListView("files");
  }
  function locationName(name?: string, path?: string) {
    if (name && name !== "/") return name;
    const known = app.targets.find((item) => item.path === path)?.name;
    return known && known !== "/"
      ? known
      : path === "/"
        ? "Startup disk"
        : name || "Storage explorer";
  }

  const queue = $derived(app.queue);
  const queueOpen = $derived(app.queueOpen);
  const queueBusy = $derived(app.queueBusy);
  const clearQueueOpen = $derived(app.clearQueueOpen);
  const performanceBuild = import.meta.env.VITE_PERFORMANCE_BENCHMARK === "1";
  const performanceConfig = $derived(app.performanceConfig);
  const performanceEventsReady = $derived(app.performanceEventsReady);
  const performanceMapScan = $derived(app.performanceMapScan);
  const performanceMapReady = $derived(app.performanceMapReady);
  const performanceLayoutPending = app.performanceLayoutPending;
  const targets = $derived(app.targets);
  const target = $derived(app.target);
  // Home shows the welcome page while the scan stays in memory.
  const summary = $derived(app.shown?.summary ?? null);
  const failedScan = $derived(
    summary?.state === "failed" && !dismissedFailedScans.has(summary.scanId)
      ? summary
      : null,
  );
  const focus = $derived(app.focus);
  const crumbs = $derived(app.crumbs);
  const parentFolder = $derived(app.parentFolder);
  const page = $derived(app.page);
  const map = $derived(app.map);
  const resultsReturn = $derived(app.resultsReturn);
  const listScrollRestore = $derived(app.listScrollRestore);
  let hoverEpoch = $state(0);
  let hover = $state.raw<{
    source: "map" | "list";
    target: HoverTarget;
  } | null>(null);
  function updateHover(source: "map" | "list", target: HoverTarget | null) {
    if (!target || target.node.kind === "aggregate") {
      if (hover?.source === source) hover = null;
      return;
    }
    if (hover?.source === source && hover.target.node === target.node) return;
    hover = { source, target };
  }
  const colorForNode = $derived(
    createBranchColorResolver(
      focus ? [focus, ...map] : map,
      focus?.id ?? summary?.rootId ?? 0,
    ),
  );
  const selection = $derived(app.selection);
  // Derived, so progress events of the same scan do not re-run the reset below.
  const shownScanId = $derived(app.summary?.scanId);
  $effect(() => {
    shownScanId;
    untrack(() => {
      listView = "files";
      fileTypesMounted = false;
    });
  });
  const queuedPaths = $derived(queue.entries);
  const readyHereEntries = $derived(app.readyHereEntries);
  const sq = app.queueSelection;
  const cleanupGroups = $derived(app.cleanupQueue.groups);
  const queueTally = $derived(app.cleanupQueue.tally);
  const queueStatusLabels: Record<QueueEntry["status"], string> = {
    ready: "Ready",
    needsScan: "Needs scan",
    missing: "Missing",
    changed: "Changed",
    blocked: "Blocked",
  };

  const mode = $derived(app.mode);
  const query = $derived(app.query);
  const offset = $derived(app.offset);
  const error = $derived(app.error);
  const busy = $derived(app.busy);
  const cancelling = $derived(app.cancelling);
  const trash = $derived(app.trash);
  const filters = $derived(app.filters);
  const debouncedQuery = $derived(app.debouncedQuery);
  const listLoading = $derived(app.listLoading);
  const sort = $derived(app.sort);
  const sortReady = $derived(app.sortReady);
  const sortSaving = $derived(app.sortSaving);
  const sortWarning = $derived(app.sortWarning);
  const access = $derived(app.access);
  const accessOpen = $derived(app.accessOpen);
  const accessPrompt = $derived(app.accessPrompt);
  const accessBusy = $derived(app.accessBusy);
  const accessError = $derived(app.accessError);
  const accessReturned = $derived(app.accessReturned);
  const accessOff = $derived(access?.status === "off");
  let pathCopied = $state(false);
  $effect(() => {
    if (!app.accessOpen) pathCopied = false;
  });
  async function copyAccessPath(path: string) {
    try {
      await writeText(path);
      pathCopied = true;
    } catch (e) {
      app.setAccessError(String(e));
    }
  }
  const dialogOpen = $derived(
    accessOpen ||
      !!trash ||
      clearQueueOpen ||
      !!app.cleanupReport ||
      !!failedScan ||
      !!error,
  );
  const quickLook = $derived(app.quickLook);
  const filtered = $derived(app.filtered);
  const filterKey = $derived(app.filterKey);
  const resultLocation = $derived(app.resultLocation);
  const displayedTargetSelected = $derived(app.displayedTargetSelected);
  const scanActionLabel = $derived(
    displayedTargetSelected
      ? `Rescan ${locationName(resultLocation?.name, resultLocation?.path)}`
      : `Scan ${locationName(target?.name, target?.path)}`,
  );
  const filterLocation = $derived(
    filters.scopeId == null
      ? resultLocation
      : (crumbs.find((node) => node.id === filters.scopeId) ?? focus),
  );
  const folderSearch = $derived(
    filtered && filters.scopeId != null && filters.scopeId !== summary?.rootId,
  );
  const searchLocation = $derived(
    folderSearch ? filterLocation : resultLocation,
  );
  const blocked = $derived(app.blocked);
  const running = $derived(app.running);
  const sessionBlocked = $derived(app.sessionBlocked);
  const previewDialog = $derived(app.previewDialog);
  const previewNode = $derived(app.previewNode);
  const previewVisible = $derived(app.previewVisible);
  const inspected = $derived(app.inspected);
  const detailsSubject = $derived(
    selection.length > 1 ? null : (selection[0] ?? inspected),
  );
  const detailsName = $derived(
    selection.length > 1
      ? `${selection.length.toLocaleString()} items selected`
      : detailsSubject?.name || "Select an item",
  );
  const detailsSize = $derived(
    !detailsSubject
      ? ""
      : detailsSubject.excluded
        ? "Excluded"
        : compactBytes(detailsSubject.allocatedBytes),
  );
  const setMode = app.setMode;
  const setOffset = app.setOffset;
  const setError = app.setError;
  const setCancelling = app.setCancelling;
  const setTrash = app.setTrash;
  const navigation = app.navigation;
  $effect(() => {
    // Hover belongs to one displayed result, never to a future scan or query.
    summary?.scanId;
    focus?.id;
    mode;
    query;
    offset;
    filterKey;
    sort;
    map;
    page.items;
    queueOpen;
    untrack(() => {
      hover = null;
      hoverEpoch++;
    });
  });
  const invalidateList = app.invalidateList;
  const changeFilters = app.changeFilters;
  const persistSort = app.persistSort;
  const changeSort = app.changeSort;
  // Clicking a column sorts by it in its natural direction; clicking again reverses.
  function sortBy(column: "name" | "size") {
    const [first, flip] =
      column === "name"
        ? (["nameAscending", "nameDescending"] as const)
        : (["sizeDescending", "sizeAscending"] as const);
    changeSort(sort === first ? flip : first);
  }
  const previewEligible = app.previewEligible;
  const previewRow = app.previewRow;
  const fail = app.fail;
  const performanceRendered = app.performanceRendered;
  const start = app.start;
  const open = app.open;
  const select = app.select;
  const prepareTrash = app.prepareTrash;
  const commitTrash = app.commitTrash;
  let searchInput = $state<HTMLInputElement>();
  function cancelScan() {
    const scan = app.summary;
    if (!scan) return;
    setCancelling(true);
    invoke("cancel_scan", { scanId: scan.scanId }).catch((e) => {
      setCancelling(false);
      fail(e);
    });
  }
  function revealInFinder() {
    const scan = app.summary;
    if (!scan) return;
    invoke("reveal_node", {
      scanId: scan.scanId,
      nodeId: inspected?.id,
    }).catch(fail);
  }
  // A single folder keeps the Quick Look button in place, greyed out, so the row doesn't reflow.
  const folderOnly = $derived(
    selection.length <= 1 && inspected?.kind === "directory",
  );
  const quickLookLabel = $derived(
    quickLook.pending
      ? "Opening Quick Look…"
      : previewVisible
        ? "Close Quick Look"
        : "Quick Look",
  );
  function toggleQuickLook() {
    if (previewVisible) quickLook.dismiss();
    else if (app.summary && previewNode && previewEligible(previewNode))
      void quickLook.open({
        scanId: app.summary.scanId,
        nodeId: previewNode.id,
      });
  }
  // Review always goes through the confirmation dialog, whether from the button or the menu.
  function reviewTrash() {
    // Only the menu reaches here; focus returns to what held it, not an earlier button.
    const active = document.activeElement;
    dialogOpener =
      active instanceof HTMLElement && active !== document.body ? active : null;
    trashSelectedCount = selection.length;
    void prepareTrash();
  }
  function showListViewFromMenu(view: typeof listView) {
    listVisible = true;
    showListView(view);
  }
  function focusSearch() {
    listVisible = true;
    searchInput?.focus();
    searchInput?.select();
  }

  const menuCommands: MenuCommands = {
    chooseFolder: () => app.chooseFolder(),
    rescan: () => app.rescan(),
    scanAllDisks: () => app.scanAllDisks(),
    stopScan: cancelScan,
    find: focusSearch,
    sunburst: () => setMode("sunburst"),
    treemap: () => setMode("treemap"),
    viewFiles: () => showListViewFromMenu("files"),
    viewFileTypes: () => showListViewFromMenu("fileTypes"),
    toggleSidebar,
    toggleFileList,
    guardianTheme: () => applyGuardian(!theme.guardian),
    back: () => app.goBack(),
    forward: () => app.goForward(),
    enclosing: () => app.can.enclosing && parentFolder && open(parentFolder),
    openFolder: () =>
      app.can.openFolder && selection.length === 1 && open(selection[0]),
    cleanupQueue: () => app.showQueue(true),
    quickLook: toggleQuickLook,
    reveal: revealInFinder,
    addToQueue: () => app.can.addToQueue && app.queueSelection.add(),
    trash: reviewTrash,
  };
  // Modal dialogs, loading screens and the filters popover own the keyboard.
  const menuQuiet = $derived(
    dialogOpen || filtersOpen,
  );
  let typing = $state(false);
  const menuState = $derived<MenuState>({
    enabled: {
      chooseFolder: app.can.chooseFolder,
      rescan: app.can.rescan,
      scanAllDisks: app.can.scanAllDisks,
      stopScan: app.can.stopScan,
      find: app.can.find,
      sunburst: app.can.explore,
      treemap: app.can.explore,
      viewFiles: app.can.explore,
      viewFileTypes: app.can.explore,
      toggleSidebar: app.can.toggleSidebar,
      toggleFileList: app.can.toggleFileList,
      guardianTheme: true,
      back: app.can.goBack,
      forward: app.can.goForward,
      enclosing: app.can.enclosing,
      openFolder: app.can.openFolder,
      cleanupQueue: app.can.cleanupQueue,
      quickLook: app.can.quickLook,
      reveal: app.can.reveal,
      addToQueue: app.can.addToQueue,
      trash: app.can.trash,
    },
    checked: {
      sunburst: mode === "sunburst",
      treemap: mode === "treemap",
      viewFiles: listView === "files",
      viewFileTypes: listView === "fileTypes",
      guardianTheme: theme.guardian,
    },
    quiet: menuQuiet,
    typing,
    sidebarVisible: locationsSidebarVisible,
    fileListVisible: listVisible,
  });
  let appMenu = $state.raw<AppMenu | null>(null);
  onMount(() => {
    if (!isTauri()) return;
    let mounted = true;
    createAppMenu(menuCommands)
      .then((menu) => {
        if (mounted) appMenu = menu;
      })
      .catch(console.error);
    return () => {
      mounted = false;
    };
  });
  $effect(() => {
    void appMenu?.sync(menuState);
  });
</script>

{#snippet cleanupRow({ entry, where }: FolderRow)}
  <li class="cleanup-entry" data-status={entry.status}>
    <div class="cleanup-entry-copy">
      <div class="cleanup-entry-heading">
        <strong>{entry.name}</strong>
        <span class="path" title={entry.path}>{where ? `in ${where}/` : ""}</span>
      </div>
      {#if entry.status !== "ready"}<p class="cleanup-entry-reason">
          {entry.reason ||
            "Scan this location to review this candidate."}
        </p>{/if}
      {#if entry.allocatedBytes === 0}<p
          class="cleanup-entry-reason"
        >
          0 B on disk — empty, or all its blocks are shared with
          clones
        </p>{/if}
      {#if entry.lastError}<p
          class="cleanup-entry-error"
          role="alert"
        >
          Last cleanup error: {entry.lastError}
        </p>{/if}
    </div>
    <span class="queue-status" data-status={entry.status}
      >{queueStatusLabels[entry.status]}</span
    >
    <span class="cleanup-entry-sizes"
      ><span
        class="cleanup-figure"
        title={bytes(entry.allocatedBytes)}
        >{compactBytes(entry.allocatedBytes)}</span
      ></span
    >
    <button
      class="cleanup-remove"
      disabled={blocked || queueBusy || queue.blocked}
      title="Remove from the queue only; keep the file in place"
      aria-label={`Remove ${entry.path} from cleanup queue`}
      onclick={() => void app.removeQueue(entry.id)}
      ><Icon name="close" size={14} /></button
    >
  </li>
{/snippet}

{#snippet navigationButtons()}
  <div
    class="toolbar-navigation"
    role="group"
    aria-label="Folder navigation"
    data-tauri-drag-region="false"
  >
    <button
      aria-label="Back"
      title="Back"
      disabled={!app.can.goBack}
      onclick={() => void app.goBack()}
    >
      <Icon name="chevron" size={18} class="back-chevron" />
    </button>
    <button
      aria-label="Forward"
      title="Forward"
      disabled={!app.can.goForward}
      onclick={() => void app.goForward()}
    >
      <Icon name="chevron" size={18} />
    </button>
  </div>
{/snippet}

{#snippet fileList()}
  {#if summary}
    <section
      class="file-list"
      id="file-list-panel"
      aria-label="Folders and files"
      style={`--row-scrollbar-width: ${listScrollbarWidth}px`}
    >
      <div class="panel-title">
        <div class="list-heading">
          <strong>
            {filtered
              ? `Filtered results across ${locationName(filterLocation?.name, filterLocation?.path)}`
              : query.trim()
                ? `Search results across ${locationName(resultLocation?.name, resultLocation?.path)}`
                : "Folders and files"}{" "}<small>
              {page.total.toLocaleString()} items
            </small>
          </strong>
          {#if filtered || query.trim()}
            <small
              class="results-scope"
              id="results-scope"
              title={folderSearch
                ? `Current folder: ${filterLocation?.path ?? ""}`
                : `Entire scan: ${crumbs[0]?.path ?? ""}`}
            >
              {folderSearch
                ? `Current folder · ${locationName(filterLocation?.name, filterLocation?.path)}`
                : `Entire scan · ${locationName(crumbs[0]?.name, crumbs[0]?.path)}`}
            </small>
          {/if}
        </div>
        <div class="list-controls">
          <SortControl
            ready={sortReady}
            saving={sortSaving}
            warning={sortWarning}
            onRetry={() => void persistSort(sort)}
          />
          <div class="filters-control">
            <button
              class:chosen={filtered}
              aria-expanded={filtersOpen}
              aria-controls="explorer-filters"
              onclick={toggleFilters}
              ><Icon name="filter" size={16} /> Filters{filtered
                ? " · Active"
                : ""}</button
            >
            <div
              class="filters-popover"
              style={filtersStyle}
              id="explorer-filters"
              role="region"
              aria-label="Filters"
              hidden={!filtersOpen}
              use:dismissPopover={() => (filtersOpen = false)}
            >
              {#key `${summary.scanId}:${navigation.current}`}
                <ExplorerFilters
                  value={filters}
                  disabled={blocked}
                  onChange={changeFilters}
                />
              {/key}
            </div>
          </div>
        </div>
      </div>
      <ActiveFilters
        value={filters}
        disabled={blocked}
        onChange={changeFilters}
      />
      {#if listLoading || !sortReady}
        <p role="status" class="filter-list-status">Loading results…</p>
      {/if}
      <div class="table-head" data-sort={sort}>
        <span>
          <button
            class="sort-heading"
            aria-label="Sort by name"
            title={sortLabels[sort.startsWith("name") ? sort : "nameAscending"]}
            disabled={!sortReady || sortSaving}
            onclick={() => sortBy("name")}
            >{filtered || query.trim() ? "Name / Location" : "Name"}{#if sort.startsWith("name")}<span
                aria-hidden="true">{sort === "nameAscending" ? " ↑" : " ↓"}</span
              >{/if}</button
          >
        </span>
        <span
          class="relative-size-heading"
          title="Share of the entire scanned size, including when filtering"
          >% of scan</span
        >
        <span class="size-heading">
          <button
            class="sort-heading"
            aria-label="Sort by size"
            title={sortLabels[sort.startsWith("size") ? sort : "sizeDescending"]}
            disabled={!sortReady || sortSaving}
            onclick={() => sortBy("size")}
            >{#if sort.startsWith("size")}<span aria-hidden="true"
                >{sort === "sizeDescending" ? "↓ " : "↑ "}</span
              >{/if}On disk</button
          >
        </span>
        <span class="contents-heading"> Contents </span>
      </div>
      {#key `${summary.scanId}:${focus?.id}:${offset}:${debouncedQuery}:${filterKey}:${sort}`}
        <FileRows
          revision={app.revision}
          onLoadChildren={filtered || query.trim()
            ? undefined
            : (nodeId) =>
                invoke<Page>("get_children", {
                  scanId: summary.scanId,
                  nodeId,
                  offset: 0,
                  limit: PAGE_SIZE,
                  sort,
                })}
          items={page.items}
          showParentPaths={filtered || !!query.trim()}
          {colorForNode}
          {queuedPaths}
          hovered={hover?.target ?? null}
          onHover={(target) => updateHover("list", target)}
          selection={selection.map((n) => n.id)}
          selectionPath={selection.length === 1 ? selection[0].path : null}
          scanBytes={summary.allocatedBytes}
          running={!!running || listLoading || !sortReady}
          onSelect={select}
          onSelectRange={app.selectRange}
          onOpen={open}
          onPreview={previewRow}
          keyboardBlocked={previewDialog}
          onScrollbarWidthChange={(width) => (listScrollbarWidth = width)}
          restoreScroll={listScrollRestore}
          scrollLoading={listLoading || !sortReady}
          onScrollChange={app.setListScroll}
          onScrollRestored={app.finishListScrollRestore}
        />
      {/key}
      {#if offset > 0 || page.total > PAGE_SIZE}<footer class="pagination">
          <span>
            {page.total
              ? `${offset + 1}–${Math.min(offset + PAGE_SIZE, page.total)} of ${page.total.toLocaleString()}`
              : "0 items"}
          </span>
          <button
            disabled={listLoading || offset === 0}
            onclick={() => {
              invalidateList();
              setOffset(Math.max(0, offset - PAGE_SIZE));
            }}
          >
            Previous
          </button>
          <button
            disabled={listLoading || offset + PAGE_SIZE >= page.total}
            onclick={() => {
              invalidateList();
              setOffset(offset + PAGE_SIZE);
            }}
          >
            Next
          </button>
        </footer>{/if}
    </section>
  {/if}
{/snippet}

{#snippet detailsStrip()}
  <section
    class="inspector details-strip"
    id="selection-inspector"
    aria-label={selection.length
      ? "Selection details"
      : "Current folder details"}
  >
    <div class="details-head">
      <span
        class="details-icon"
        style:color={detailsSubject ? colorForNode(detailsSubject) : undefined}
        ><Icon
          name={detailsSubject?.kind === "directory" ? "folder" : "file"}
          size={20}
        /></span
      >
      <strong class="details-name" title={detailsSubject?.path ?? detailsName}
        >{detailsName}</strong
      >
      {#if detailsSize}
        <span class="details-size"
          >{detailsSize}
          <small>on disk</small></span
        >
      {/if}
    </div>
    <div class="inspector-actions">
      {#if selection.length > 1}
        <small class="selection-action-scope" id="selection-action-scope"
          >Actions apply to all {selection.length} selected items.</small
        >
      {/if}
      {#if quickLook.error}
        <p class="quick-look-error" role="alert">
          {quickLook.error}<button onclick={quickLook.retryClose}>
            Dismiss preview
          </button>
        </p>
      {/if}
      <div class="details-primary">
        {#if sq.removing}
          <button class="primary" disabled aria-busy="true">
            <Icon name="minus" size={17} /> Removing…
          </button>
        {:else if sq.directlyQueued}
          <button
            class="primary queue-toggle queue-added"
            disabled={blocked || queueBusy || queue.blocked}
            aria-label={`Added to cleanup queue. ${
              selection.length === 1
                ? "Remove from cleanup queue"
                : "Remove selected items from queue"
            }`}
            onclick={() => void sq.remove()}
          >
            <span class="queue-toggle-labels" aria-hidden="true">
              <span class="queue-added-label">
                <Icon name="check" size={17} /> Added to cleanup queue
              </span>
              <span class="queue-remove-label">
                <Icon name="minus" size={17} />
                {selection.length === 1
                  ? "Remove from cleanup queue"
                  : "Remove selected items from queue"}
              </span>
            </span>
          </button>
        {:else if sq.queued}
          <div class="queue-included">
            <Icon name="check" size={17} /> Added to cleanup queue
          </div>
        {:else}
          <button
            disabled={!app.can.addToQueue}
            class="primary"
            aria-busy={sq.adding}
            aria-describedby={selection.length > 1
              ? "selection-action-scope"
              : undefined}
            onclick={() => void sq.add()}
          >
            <Icon name="queue-add" size={20} />
            {sq.adding
              ? "Adding…"
              : sq.queuedCount > 0
                ? "Add remaining to cleanup queue"
                : "Add to cleanup queue"}
          </button>
        {/if}
        <button
          class="icon-action"
          aria-label="Reveal in Finder"
          title="Reveal in Finder"
          disabled={!app.can.reveal}
          onclick={revealInFinder}
        >
          <Icon name="folder" size={16} />
        </button>
        <button
          class="icon-action"
          aria-label={quickLookLabel}
          title={folderOnly
            ? "Quick Look previews files only"
            : quickLookLabel}
          disabled={folderOnly || !app.can.quickLook}
          onclick={toggleQuickLook}
        >
          <Icon name="eye" size={16} />
        </button>
      </div>
      {#if !sq.directlyQueued && sq.entries.length > 0 && !sq.removing}
        <button
          class="queue-remove-partial"
          disabled={blocked || queueBusy || queue.blocked}
          onclick={() => void sq.remove()}
        >
          <Icon name="minus" size={17} /> Remove {sq.entries.length}
          queued {sq.entries.length === 1 ? "item" : "items"}
        </button>
      {/if}
      <small
        class="queue-feedback"
        class:queue-confirmed={sq.queued &&
          !sq.adding &&
          !sq.removing &&
          !sq.removalFeedback}
        hidden={!sq.feedback}
        role="status"
        aria-live="polite"
      >
        {sq.feedback}
      </small>
      {#if sq.inheritedCount > 0}
        <small class="queue-inherited-feedback">
          {selection.length === 1
            ? "This item is included through a queued parent folder. Review the queue to remove that folder."
            : `${sq.inheritedCount} selected ${sq.inheritedCount === 1 ? "item is" : "items are"} included through queued parent folders. Review the queue to remove those folders.`}
        </small>
        <button
          class="queue-review"
          disabled={!app.can.cleanupQueue}
          onclick={() => app.showQueue(true)}>Review queue</button
        >
      {/if}
      {#if app.queueError}<p role="alert">{app.queueError}</p>{/if}
    </div>
  </section>
{/snippet}

{#snippet listColumn()}
  {#if summary}
    <aside
      class="workspace-side"
      id="file-list-column"
      aria-label="File list and details"
      hidden={!listVisible}
    >
      <div
        class="segmented list-view-tabs"
        role="tablist"
        aria-label="List view"
      >
        {#each listViews as tab}
          <button
            role="tab"
            id={tab.id}
            aria-controls={tab.panel}
            aria-selected={listView === tab.name}
            tabindex={listView === tab.name ? 0 : -1}
            onclick={() => showListView(tab.name)}
            onkeydown={listViewKey}>{tab.label}</button
          >
        {/each}
      </div>
      <div
        class="list-view-panel"
        id="files-panel"
        role="tabpanel"
        aria-labelledby="files-tab"
        hidden={listView !== "files"}
      >
        {@render fileList()}
      </div>
      <div
        class="list-view-panel"
        id="file-types-panel"
        role="tabpanel"
        aria-labelledby="file-types-tab"
        hidden={listView !== "fileTypes"}
      >
        {#if fileTypesMounted}
          <FileTypes
            stats={fileTypes.stats}
            scope={fileTypes.scope}
            selectedExtension={fileTypes.extension}
            loading={fileTypes.loading}
            error={fileTypes.error}
            highlightPending={fileTypes.highlightsLoading}
            highlightError={fileTypes.highlightError}
            disabled={blocked}
            running={summary.state === "running"}
            scopePath={fileTypes.scope === "folder"
              ? focus?.path
              : resultLocation?.path}
            onScopeChange={fileTypes.setScope}
            onSelect={fileTypes.select}
            onShowFiles={showFileTypeFiles}
            onRetry={fileTypes.retry}
          />
        {/if}
      </div>
      {@render detailsStrip()}
    </aside>
  {/if}
{/snippet}

<svelte:window
  onresize={() => (filtersOpen = false)}
  onfocusin={(event) => (typing = isTextField(event.target))}
  onfocusout={() => (typing = false)}
/>

<div
  class="app"
  class:macos-titlebar={macosTitlebar}
  class:navigation-sidebar-hidden={!locationsSidebarVisible}
  style:--sidebar-width={locationsSidebarVisible && sidebarWidth
    ? `min(${sidebarWidth}px, 40vw)`
    : undefined}
  use:captureDialogOpener={(element) => (dialogOpener = element)}
  data-native-busy={listLoading}
  data-native-map-mode={mode}
  data-native-map-ready={performanceBuild ? performanceMapReady : undefined}
  data-native-focus={focus?.id}
>
  <button
    class="titlebar-sidebar-toggle"
    data-tauri-drag-region="false"
    aria-label={locationsSidebarVisible
      ? "Hide sidebar"
      : "Show sidebar"}
    title={locationsSidebarVisible
      ? "Hide sidebar"
      : "Show sidebar"}
    aria-controls="locations-sidebar"
    aria-expanded={locationsSidebarVisible}
    disabled={!app.can.toggleSidebar}
    onclick={toggleSidebar}><Icon name="sidebar-left" size={19} /></button
  >
  <Sidebar
    {app}
    {macosTitlebar}
    visible={locationsSidebarVisible}
    inert={!!failedScan || !!error}
  />
  {#if locationsSidebarVisible}
    <SidebarDivider
      value={sidebarWidth}
      onChange={(value) => (sidebarWidth = value)}
    />
  {/if}
  <main
    onscroll={() => (filtersOpen = false)}
    inert={dialogOpen}
  >
    {#if !summary || queueOpen}
      <div class="window-toolbar-fallback" data-tauri-drag-region="deep">
        {@render navigationButtons()}
        <strong>Hahakigami</strong>
      </div>
    {/if}
    {#if queueOpen}
      <section class="cleanup-page" aria-labelledby="cleanup-title">
        <header class="cleanup-head">
          <h1 id="cleanup-title">Cleanup queue</h1>
          <p class="cleanup-tally">{queueTally}</p>
          <button class="cleanup-back" onclick={() => app.showQueue(false)}
            >Back to explorer</button
          >
        </header>
        {#if queue.warning}<p class="cleanup-alert" role="alert">
            {queue.warning}
          </p>{/if}
        {#if app.queueError}<p class="cleanup-alert" role="alert">
            {app.queueError}
            <button disabled={queueBusy} onclick={() => void app.refreshQueue()}
              >Retry</button
            >
          </p>{/if}
        {#if !queue.entries.length && !queue.blocked}<div class="cleanup-empty">
            <Icon name="queue" size={28} />
            <p>
              Your cleanup queue is empty. Select files or folders in a
              completed scan to add them.
            </p>
          </div>{/if}
        {#each cleanupGroups as group (group.root)}
          <section class="cleanup-group" aria-labelledby={group.titleId}>
            <header class="cleanup-group-head">
              <Icon name="disk" size={16} />
              <h2 id={group.titleId}>{locationName(group.name, group.root)}</h2>
              {#if group.root !== "/"}<span class="path">{group.root}</span>{/if}
              <span class="cleanup-figure" title={bytes(group.allocatedBytes)}
                >{compactBytes(group.allocatedBytes)}</span
              >
              <div class="cleanup-group-actions">
                {#if group.readyHere}<button
                    class="primary cleanup-primary"
                    aria-describedby="queue-safety"
                    disabled={!app.can.reviewHere}
                    onclick={() => void app.prepareQueue()}
                    >{readyHereEntries.length > group.readyHere
                      ? `Review all ${readyHereEntries.length} ready`
                      : `Review ${group.readyHere} ready`}</button
                  >{/if}
                {#if group.unreviewable}<button
                    class:primary={!group.readyHere && !group.onlyElsewhere}
                    class:cleanup-primary={!group.readyHere && !group.onlyElsewhere}
                    disabled={queueBusy ||
                      app.queueTargeting ||
                      blocked ||
                      app.queueRootBusy(group.root) ||
                      queue.blocked}
                    onclick={() => void app.scanQueue(group.entries[0].id)}
                    >Scan {locationName(group.name, group.root)}</button
                  >{/if}
              </div>
            </header>
            {#if group.segments.length}
              <div
                class="cleanup-band"
                aria-hidden="true"
                style:width="{group.share * 100}%"
              >
                {#each group.segments as segment (segment.kind)}
                  <span
                    class="cleanup-band-segment"
                    data-kind={segment.kind}
                    style:flex-grow={segment.bytes}
                    >{#if segment.showLabel}{segment.label}{/if}</span
                  >
                {/each}
              </div>
              <p class="sr-only">{group.bandSummary}</p>
            {/if}
            {#if group.base}<p class="cleanup-base">
                in <span class="path" title={group.base}
                  ><bdi>{group.base}/</bdi></span
                >
              </p>{/if}
            <ul class="cleanup-rows">
              {#each group.items as item (item.kind === "row" ? item.row.entry.id : `folder:${item.name}`)}
                {#if item.kind === "row"}
                  {@render cleanupRow(item.row)}
                {:else}
                  <li class="cleanup-folder">
                    <div class="cleanup-folder-head">
                      <Icon name="folder" size={14} />
                      <strong>{item.name}/</strong>
                      <span class="cleanup-folder-count"
                        >{item.rows.length} items</span
                      >
                      <span
                        class="cleanup-figure"
                        title={bytes(item.allocatedBytes)}
                        >{compactBytes(item.allocatedBytes)}</span
                      >
                      <button
                        class="cleanup-remove"
                        disabled={blocked || queueBusy || queue.blocked}
                        title="Remove these items from the queue only; keep the files in place"
                        aria-label={`Remove ${item.rows.length} items in ${item.name} from cleanup queue`}
                        onclick={() =>
                          void app.removeQueues(
                            item.rows.map((row) => row.entry.id),
                          )}><Icon name="close" size={14} /></button
                      >
                    </div>
                    <ul class="cleanup-rows">
                      {#each item.rows as row (row.entry.id)}
                        {@render cleanupRow(row)}
                      {/each}
                    </ul>
                  </li>
                {/if}
              {/each}
            </ul>
          </section>
        {/each}
        {#if queue.entries.length || queue.blocked}<footer class="cleanup-foot">
            <p id="queue-safety">
              Nothing moves until you confirm. Removing only edits this list;
              the space you get back may differ.
            </p>
            <button
              class="cleanup-clear"
              disabled={blocked ||
                queueBusy ||
                (!queue.entries.length && !queue.blocked)}
              onclick={() => app.setClearQueueOpen(true)}>Clear queue</button
            >
          </footer>{/if}
      </section>
    {/if}

    {#if !queueOpen}
      {#if resultsReturn && summary}
        <div
          class="results-return-context"
          aria-label="Return to search results"
        >
          <button
            aria-label="Back to results"
            disabled={sessionBlocked || previewDialog}
            onclick={() => {
              listVisible = true;
              showListView("files");
              app.returnToResults();
            }}>← Back to results</button
          >
          <span title={resultsReturn.query.trim() || "Filtered results"}>
            {resultsReturn.query.trim()
              ? `“${resultsReturn.query.trim()}”`
              : "Filtered results"}
            · {resultsReturn.total.toLocaleString()} matches
          </span>
        </div>
      {/if}
      {#if summary}
        <header class="window-toolbar" data-tauri-drag-region="deep">
          {@render navigationButtons()}
          <nav class="toolbar-crumbs" aria-label="Folder breadcrumbs">
            {#each crumbs as n, i (n.id)}
              {#if i > 0}<Icon name="chevron" size={12} />{/if}
              <button
                title={i === 0 ? (resultLocation?.path ?? n.path) : n.path}
                aria-current={i === crumbs.length - 1 ? "page" : undefined}
                onclick={() => open(n)}
              >
                {i === 0
                  ? locationName(resultLocation?.name, resultLocation?.path)
                  : n.name}
              </button>
            {/each}
          </nav>
          <div class="header-controls" data-tauri-drag-region="false">
            <label class="header-search"
              ><Icon name="search" size={17} />
              <input
                bind:this={searchInput}
                aria-label="Search file names"
                aria-describedby={filtered || query.trim()
                  ? "search-scope-description results-scope"
                  : "search-scope-description"}
                title={`Search file names ${folderSearch ? "in the filtered folder" : "across the entire scan"}: ${searchLocation?.path ?? locationName(searchLocation?.name, searchLocation?.path)}`}
                placeholder={folderSearch
                  ? `Search ${locationName(searchLocation?.name, searchLocation?.path)}…`
                  : "Search entire scan…"}
                value={query}
                oninput={(e) => {
                  listVisible = true;
                  showListView("files");
                  app.changeQuery(e.currentTarget.value);
                }}
              />
            </label>
            <span id="search-scope-description" class="sr-only">
              {folderSearch
                ? `Search stays within ${searchLocation?.path ?? "the filtered folder"} and the active filters. Clear filters to search the entire scan.`
                : `Search covers all file names in ${locationName(resultLocation?.name, resultLocation?.path)}, including folders outside the current view.`}
            </span>
            <!-- Rescan and Stop share one button so focus stays put when a scan starts or ends. -->
            <button
              class="rescan-action"
              class:primary={running}
              aria-label={running ? undefined : scanActionLabel}
              title={running ? "Stop scan" : scanActionLabel}
              disabled={running ? !app.can.stopScan : !app.can.rescan}
              onclick={(e) => {
                // Rescan turns into Stop under the pointer; a double-click must not cancel.
                if (e.detail > 1) return;
                if (running) cancelScan();
                else void app.rescan();
              }}
            >
              {#if running}
                <span>{cancelling ? "Stopping…" : "Stop"}</span>
              {:else}
                <Icon name="refresh" size={16} /><span
                  >{displayedTargetSelected ? "Rescan" : "Scan"}</span
                >
              {/if}
            </button>
            <button
              class="toolbar-sidebar-toggle"
              aria-label={listVisible ? "Hide file list" : "Show file list"}
              title={listVisible ? "Hide file list" : "Show file list"}
              aria-controls="file-list-column"
              aria-expanded={listVisible}
              disabled={!app.can.toggleFileList}
              onclick={toggleFileList}
              ><Icon name="sidebar-right" size={19} /></button
            >
          </div>
        </header>
        {#snippet viewControls()}
          <div class="workspace-controls">
            <span class="map-control-label">View:</span>
            <div
              class="segmented view-switch"
              role="group"
              aria-label="Map view"
            >
              <button
                aria-label="Treemap"
                class:chosen={mode === "treemap"}
                aria-pressed={mode === "treemap"}
                onclick={() => setMode("treemap")}
              >
                <Icon name="treemap" size={16} /> Treemap
              </button>
              <button
                aria-label="Sunburst"
                class:chosen={mode === "sunburst"}
                aria-pressed={mode === "sunburst"}
                onclick={() => setMode("sunburst")}
              >
                <Icon name="sunburst" size={16} /> Sunburst
              </button>
            </div>
          </div>
        {/snippet}
        <ScanStatusLine
          {summary}
          {cancelling}
          controls={viewControls}
        />
      {/if}
    {/if}
    {#if !queueOpen}
      <div class="analysis-layout" class:has-scan={!!summary}>
        <div class="analysis-main">
          <div class:has-scan={!!summary} class="explorer-content">
            {#if !summary}
              <section class="welcome" aria-labelledby="welcome-title">
                <img class="welcome-icon" src={appIcon} alt="" />
                <h1 id="welcome-title">See what’s using your space.</h1>
                <p class="welcome-intro">
                  Scan a disk or folder to find its largest files.
                </p>
                <button
                  class="welcome-target"
                  id="welcome-target"
                  title={target?.path}
                  aria-busy={app.choosingFolder}
                  disabled={!app.can.chooseFolder}
                  onclick={() => void app.chooseFolder()}
                >
                  <Icon
                    name={target?.totalBytes ? "disk" : "folder"}
                    size={28}
                  />
                  <span class="welcome-target-text">
                    <strong
                      >{target
                        ? locationName(target.name, target.path)
                        : app.targetsLoading
                          ? "Loading locations…"
                          : "No location selected"}</strong
                    >
                    <span
                      >{app.choosingFolder
                        ? "Choosing folder…"
                        : target
                          ? "Choose another location"
                          : "Choose a location"}</span
                    >
                  </span>
                </button>
                <div class="welcome-actions">
                  <button
                    class="primary welcome-scan"
                    aria-describedby="welcome-note"
                    aria-busy={!!target && app.scanStarting(target)}
                    title={target
                      ? `Scan ${locationName(target.name, target.path)}`
                      : "Scan selected location"}
                    disabled={!app.can.rescan}
                    onclick={() => void start(target)}
                    ><Icon name="refresh" size={17} /><span
                      >{target && app.scanStarting(target)
                        ? "Starting scan…"
                        : app.targetsLoading
                          ? "Loading locations…"
                          : target
                            ? `Scan ${locationName(target.name, target.path)}`
                            : "Scan location"}</span
                    ></button
                  >
                  <button
                    class="welcome-scan-all"
                    title="Scan every disk"
                    disabled={!app.can.scanAllDisks}
                    onclick={() => void app.scanAllDisks()}
                    ><Icon name="disk" size={17} /><span>Scan all disks</span
                    ></button
                  >
                </div>
                <p class="welcome-note" id="welcome-note">
                  {target || app.targetsLoading || !isTauri()
                    ? "Nothing leaves your Mac."
                    : "Choose a location to scan."}
                </p>
                {#if !isTauri()}
                  <small class="welcome-browser">
                    This is a browser preview. Open the desktop app to scan or
                    pick a folder.
                  </small>
                {/if}
              </section>
            {:else}
              <section
                class:sunburst-view={mode === "sunburst"}
                class:inspector-hidden={!listVisible}
                class="explorer"
                style={`--map-share: ${mapShare * 100}fr; --list-share: ${(1 - mapShare) * 100}fr`}
              >
                <div class="visual" id="storage-map-panel">
                  {#if filtered || query.trim()}
                    <p class="filter-map-caption" title={focus?.path}>
                      Unfiltered storage map and totals · {locationName(
                        focus?.name,
                        focus?.path,
                      )}
                    </p>
                  {/if}
                  <DiskMap
                    colorMode={mapColorMode}
                    onFileTypeSelect={(extension) => {
                      fileTypes.select(extension);
                      showListView("fileTypes");
                    }}
                    highlightedFileType={fileTypes.extension}
                    fileTypeHighlights={fileTypes.highlights}
                    fileTypeHighlightPending={fileTypes.highlightsLoading}
                    scanId={summary.scanId}
                    scanComplete={summary.state === "completed"}
                    onRendered={performanceBuild
                      ? performanceRendered
                      : undefined}
                    onLayoutPending={performanceBuild
                      ? performanceLayoutPending
                      : undefined}
                    nodes={map}
                    {queuedPaths}
                    rootId={focus?.id ?? summary.rootId}
                    {mode}
                    selected={selection.map((n) => n.id)}
                    hovered={hover?.target ?? null}
                    {hoverEpoch}
                    onHover={(target) => updateHover("map", target)}
                    onSelect={select}
                    onOpen={open}
                  />
                </div>
                {#if listVisible}
                  <PanelDivider
                    value={mapShare}
                    onChange={(value) => (mapShare = value)}
                  />
                {/if}
                {@render listColumn()}
              </section>
              {#if queue.entries.length > 0}
                <div class="queue-strip">
                  <Icon name="queue" size={19} />
                  <strong
                    >Cleanup queue <span
                      >· {queue.entries.length}
                      {queue.entries.length === 1 ? "item" : "items"}</span
                    ></strong
                  >
                  <span
                    class:queue-needs-scan={queue.entries.some(
                      (entry) => entry.status !== "ready",
                    )}
                  >
                    {queue.entries.some((entry) => entry.status !== "ready")
                      ? "Some items not ready"
                      : "Ready for review"}
                  </span>
                  <button
                    disabled={!app.can.cleanupQueue}
                    onclick={() => app.showQueue(true)}>Review queue</button
                  >
                </div>
              {/if}
            {/if}
          </div>
        </div>
      </div>
    {/if}
  </main>
  {#if accessOpen}
    <Modal
      class="access-modal"
      labelledby={accessPrompt ? "access-title" : "settings-title"}
      describedby={macosTitlebar ? "access-message" : undefined}
      initialFocus=".access-actions .primary"
      opener={dialogOpener}
      onClose={() => {
        if (!accessBusy) app.closeAccess();
      }}
    >
      {#if !accessPrompt}
        <h2 id="settings-title">Settings</h2>
        <section class="settings-section" aria-labelledby="display-title">
          <h3 id="display-title">Display</h3>
          <!-- More Display rows go here, one `.settings-row` each. -->
          <div class="settings-group">
          <div class="settings-row" role="group" aria-label="Theme">
            <span>Theme</span>
            {#each [{ label: "Mac", guardian: false }, { label: "Guardian", guardian: true }] as option (option.label)}
              <button
                class="theme-swatch"
                class:guardian-swatch={option.guardian}
                aria-label={`${option.label} theme`}
                aria-pressed={theme.guardian === option.guardian}
                onclick={() => applyGuardian(option.guardian)}
                ><i class="theme-preview"></i>{option.label}</button
              >
            {/each}
          </div>
          <div class="settings-row" role="group" aria-label="Appearance">
            <span
              >Appearance{#if theme.guardian}<small
                  >Guardian is light only for now.</small
                >{/if}</span
            >
            <div class="segmented">
              {#each [{ label: "Light", value: "light" }, { label: "Dark", value: "dark" }, { label: "System", value: "system" }] as const as option (option.value)}
                <button
                  class:chosen={theme.appearance === option.value}
                  aria-pressed={theme.appearance === option.value}
                  disabled={theme.guardian}
                  onclick={() => applyAppearance(option.value)}
                  >{option.label}</button
                >
              {/each}
            </div>
          </div>
          </div>
        </section>
      {/if}
      {#if macosTitlebar}
        <svelte:element
          this={accessPrompt ? "div" : "section"}
          class="settings-section"
          class:prompt={accessPrompt}
          aria-labelledby={accessPrompt ? undefined : "access-section"}
        >
          {#if !accessPrompt}
            <h3 id="access-section">Full Disk Access</h3>
          {/if}
          <div class="access-head" class:settings-group={!accessPrompt}>
            <img class="access-icon" src={appIcon} alt="" />
            <div>
              <svelte:element
                this={accessPrompt ? "h2" : "h4"}
                id="access-title"
              >
                {#if accessOff}
                  {accessReturned
                    ? "Full Disk Access is still off"
                    : "Full Disk Access is off"}<span
                    class="access-warning-mark"
                    ><Icon name="warning" size={20} /></span
                  >
                {:else if access?.status === "on"}
                  Full Disk Access is on
                {:else}
                  Couldn’t check Full Disk Access
                {/if}
              </svelte:element>
              <p id="access-message" class:access-callout={accessOff}>
                {#if accessOff}
                  {accessReturned
                    ? "If you just turned it on, macOS applies it when Hahakigami relaunches."
                    : "Without it, macOS asks about each protected folder during a scan."}
                {:else if access?.status === "on"}
                  Hahakigami can scan protected folders without macOS asking
                  about each one.
                {:else}
                  Scans aren’t blocked. If macOS asks about protected folders,
                  turn on Full Disk Access in System Settings.
                {/if}
              </p>
              {#if accessOff}
                <p class="access-steps">
                  {#if accessReturned}
                    Not turned on yet?
                    <button
                      class="access-link"
                      disabled={accessBusy}
                      onclick={() => void app.openSettings()}
                      >Open System Settings…</button
                    >
                  {:else}
                    Turn on Hahakigami in <strong
                      >Privacy &amp; Security › Full Disk Access</strong
                    >. If it isn’t listed, add it with + using this path:
                  {/if}
                </p>
                {#if access?.appPath}
                  <div class="access-path-row">
                    <!-- Break only after slashes, never mid-folder-name. -->
                    <code class="access-path"
                      >{#each access.appPath.split("/") as part, i}{#if i > 0}/<wbr
                          />{/if}{part}{/each}</code
                    >
                    <button onclick={() => void copyAccessPath(access.appPath)}>
                      {pathCopied ? "Copied" : "Copy"}
                    </button>
                  </div>
                {/if}
              {/if}
              {#if accessError}
                <p role="alert" class="access-error">
                  {accessError}
                </p>
              {/if}
            </div>
          </div>
        </svelte:element>
      {/if}
      <div class="access-actions">
        {#if !macosTitlebar}
          <button class="primary" onclick={() => app.closeAccess()}>
            Done
          </button>
        {:else if accessOff}
          {#if accessPrompt}
            <button
              class="leading"
              disabled={accessBusy}
              onclick={() => void app.scanAnyway()}
            >
              Scan Anyway
            </button>
          {/if}
          <button disabled={accessBusy} onclick={() => app.closeAccess()}>
            {accessPrompt ? "Cancel" : "Done"}
          </button>
          {#if accessReturned}
            <button
              class="primary"
              disabled={accessBusy}
              onclick={() => void app.relaunch()}
            >
              Relaunch
            </button>
          {:else}
            <button
              class="primary"
              disabled={accessBusy}
              onclick={() => void app.openSettings()}
            >
              Open System Settings…
            </button>
          {/if}
        {:else}
          <button disabled={accessBusy} onclick={() => void app.openSettings()}>
            Open System Settings…
          </button>
          <button
            class="primary"
            disabled={accessBusy}
            onclick={() => app.closeAccess()}
          >
            Done
          </button>
        {/if}
      </div>
    </Modal>
  {/if}
  {#if clearQueueOpen}
    <div class="modal-backdrop">
      <div
        class="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="clear-queue-title"
        tabindex="-1"
        use:dialogFocus={{
          opener: dialogOpener,
          onClose: () => {
            if (!queueBusy) app.setClearQueueOpen(false);
          },
        }}
      >
        <h2 id="clear-queue-title">Clear cleanup queue?</h2>
        <p>This empties the list. Your files stay where they are.</p>
        {#if app.queueError}<p role="alert">{app.queueError}</p>{/if}
        <button
          disabled={queueBusy}
          onclick={() => app.setClearQueueOpen(false)}>Cancel</button
        ><button disabled={queueBusy} onclick={() => void app.clearQueue()}
          >Clear queue</button
        >
      </div>
    </div>
  {/if}
  <div class="toast-region" role="status" aria-live="polite" aria-atomic="true">
    {#if app.toast}
      {#key app.toast.id}
        <Toast toast={app.toast} onDismiss={app.dismissToast} />
      {/key}
    {/if}
  </div>
  {#if trash}
    <TrashConfirmation
      items={trash.items}
      queued={trash.queued}
      selectedCount={trash.queued ? undefined : trashSelectedCount}
      {busy}
      opener={dialogOpener}
      onCancel={() => setTrash(null)}
      onConfirm={() => void commitTrash()}
    />
  {/if}
  {#if app.cleanupReport}
    <CleanupResults
      report={app.cleanupReport}
      {queueOpen}
      disabled={!app.can.cleanupQueue}
      opener={dialogOpener}
      onQueue={() => {
        app.dismissCleanupReport();
        app.showQueue(true);
      }}
      onRetry={() => void app.refreshQueue()}
      onDismiss={app.dismissCleanupReport}
    />
  {/if}
  {#if error}
    <FailureModal
      title={app.errorContext?.title ?? "Something went wrong"}
      reason={app.errorContext?.reason ??
        "Hahakigami could not finish that. Try again."}
      details={error}
      opener={dialogOpener}
      onClose={() => setError("")}
    />
  {/if}
  {#if failedScan}
    <FailureModal
      title="Scan failed"
      reason="The scan stopped before it could finish. Some files or folders could not be read. Scan again to try once more."
      details={failedScan.message ?? ""}
      actionLabel="Scan again"
      actionDisabled={!app.can.rescanDisplayed}
      opener={dialogOpener}
      onAction={() => {
        dismissedFailedScans.add(failedScan.scanId);
        void app.rescanDisplayed();
      }}
      onClose={() => dismissedFailedScans.add(failedScan.scanId)}
    />
  {/if}
</div>
