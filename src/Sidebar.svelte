<script lang="ts">
  import { coverageLabel } from "./scanCoverage";
  import { isTauri } from "@tauri-apps/api/core";
  import type createApp from "./createApp.svelte";
  import Icon from "./Icon.svelte";
  import appIcon from "../src-tauri/icons/128x128.png";
  import { compactBytes } from "./uiFormat";
  import { FOLDER_SLOT, slotScanning, type ScanSlot } from "./scanSlots.svelte";
  import { logicalSize } from "./logicalSize.svelte";
  import { groupSavedScans, formatScanTimestamp } from "./savedHistory";
  import "./sidebar.css";
  import { dismissPopover } from "./dialogFocus";
  let openMenu = $state<string | null>(null);
  let menuPosition = $state({ top: 0, left: 0 });
  function toggleMenu(id: string, trigger: HTMLButtonElement) {
    if (openMenu === id) {
      openMenu = null;
      return;
    }
    const rect = trigger.getBoundingClientRect();
    menuPosition = {
      top: Math.max(8, Math.min(rect.bottom + 4, window.innerHeight - 96)),
      left: Math.max(8, Math.min(rect.right - 160, window.innerWidth - 168)),
    };
    openMenu = id;
  }
  function savedName(entry: { name: string; rootPath: string }) {
    if (entry.name !== "/") return entry.name;
    const targetName = app.targets.find(
      (target) => target.path === entry.rootPath,
    )?.name;
    return targetName && targetName !== "/" ? targetName : "Startup disk";
  }
  $effect(() => {
    if (
      !visible ||
      app.anyScanRunning ||
      app.sessionBlocked ||
      app.accessOpen ||
      app.removeEntry ||
      app.trash ||
      app.clearQueueOpen
    )
      openMenu = null;
  });
  let {
    app,
    macosTitlebar = false,
    visible = true,
    inert = false,
  }: {
    app: ReturnType<typeof createApp>;
    macosTitlebar?: boolean;
    visible?: boolean;
    // A dialog owned by the app root is open.
    inert?: boolean;
  } = $props();
  // Full Disk Access only exists on macOS; the row warns until status is On.
  const accessWarning = $derived(
    macosTitlebar && !!app.access && app.access.status !== "on",
  );
  const settingsLabel = $derived(
    !accessWarning
      ? "Settings"
      : app.access?.status === "off"
        ? "Settings: Full Disk Access is off"
        : "Settings: Full Disk Access status unknown",
  );
  const savedGroups = $derived(groupSavedScans(app.saved));
  const explorerShown = $derived(
    !app.homeOpen && !app.queueOpen && !app.comparisonOpen,
  );
  const openSavedId = $derived(
    app.historical && explorerShown ? app.summary?.savedId : undefined,
  );
  const welcomeShown = $derived(
    !app.queueOpen && !app.comparisonOpen && (app.homeOpen || !app.summary),
  );
  // The slot whose live results are on screen.
  const shownSlot = $derived(explorerShown ? (app.shown?.slot ?? null) : null);
  const folderSlot = $derived(app.scanSlots[FOLDER_SLOT]);
  // Fraction scanned, known only for a whole volume (used space is the total); null means unknown.
  function scanProgress(slot: ScanSlot, disk?: (typeof app.targets)[number]) {
    if (!slot.summary || slot.summary.state !== "running") return undefined;
    const used =
      disk?.totalBytes && disk.freeBytes != null
        ? disk.totalBytes - disk.freeBytes
        : 0;
    return used > 0 ? Math.min(0.99, slot.summary.allocatedBytes / used) : null;
  }
  let expandedLocations = $state<ReadonlySet<string>>(new Set());
  $effect(() => {
    const locations = new Set(savedGroups.map((group) => group.rootPath));
    const retained = new Set(
      [...expandedLocations].filter((path) => locations.has(path)),
    );
    if (retained.size !== expandedLocations.size) expandedLocations = retained;
  });
  function toggleHistory(path: string) {
    openMenu = null;
    const expanded = new Set(expandedLocations);
    if (expanded.has(path)) expanded.delete(path);
    else expanded.add(path);
    expandedLocations = expanded;
  }
</script>

<svelte:window onresize={() => (openMenu = null)} />

<aside
  class="sidebar compact-sidebar"
  id="locations-sidebar"
  hidden={!visible}
  inert={inert ||
    app.accessOpen ||
    !!app.removeEntry ||
    !!app.trash ||
    app.clearQueueOpen ||
    !!app.cleanupReport}
>
  {#if macosTitlebar}
    <div
      class="sidebar-titlebar"
      data-tauri-drag-region="deep"
      aria-hidden="true"
    ></div>
  {/if}
  <div class="brand">
    <span class="logo"><img src={appIcon} alt="" width="24" height="24" /></span>
    <div>Hahakigami</div>
  </div>
  <div class="side-label disks-label">
    <span>Disks</span>
    <span class="batch-status" aria-live="polite" aria-atomic="true"
      >{#if app.batchProgress}<span class="sr-only">Scanning ·</span> {app.batchProgress.done} of {app.batchProgress
          .total} done{/if}</span
    >
    <button
      class="scan-all"
      aria-label="Scan all disks"
      title="Scan all disks"
      disabled={!app.can.scanAllDisks}
      onclick={() => void app.scanAllDisks()}><Icon name="refresh" size={14} /></button
    >
  </div>
  {#each app.targets.filter((t) => t.volume) as disk (disk.id)}
    {@const share =
      disk.totalBytes && disk.freeBytes != null
        ? (disk.totalBytes - disk.freeBytes) / disk.totalBytes
        : null}
    {@const size =
      share === null
        ? "Size unknown"
        : `${compactBytes(disk.freeBytes ?? 0)} free of ${compactBytes(disk.totalBytes ?? 0)}`}
    {@const slot = app.scanSlots[disk.path]}
    {@const scanned = !!slot}
    {@const tag = slot && slotScanning(slot) ? "Scanning" : "Scanned"}
    {@const progress = slot ? scanProgress(slot, disk) : undefined}
    {@const active =
      (welcomeShown && app.target?.path === disk.path) ||
      shownSlot === disk.path}
    <button
      class="side-nav disk-nav"
      class:active
      class:scanned
      aria-current={active ? "page" : undefined}
      aria-label={`${disk.name}, ${size}${scanned ? `, ${tag}` : ""}`}
      title={`${disk.path}${scanned ? `\n${tag}` : ""}`}
      disabled={app.sessionBlocked}
      onclick={() => app.openDisk(disk)}
    >
      <Icon name="disk" size={22} /><span
        class="nav-copy"
        ><span class="disk-name"
          ><strong>{disk.name}</strong>{#if scanned}<em class:done={tag === "Scanned"}>{tag}</em>{/if}</span
        ><small>{size}</small>{#if progress !== undefined}<span
            class="scan-track"
            class:indeterminate={progress === null}
            role="progressbar"
            aria-label="Scan progress"
            aria-valuemin={progress === null ? undefined : 0}
            aria-valuemax={progress === null ? undefined : 100}
            aria-valuenow={progress === null
              ? undefined
              : Math.round(progress * 100)}
            ><span
              style:width={progress === null ? undefined : `${progress * 100}%`}
            ></span></span
          >{:else}<span
            class="disk-track"
            class:full={(share ?? 0) > 0.9}
            class:unknown={share === null}
            ><span style:width={`${(share ?? 0) * 100}%`}></span></span
          >{/if}</span
      >
    </button>
  {/each}
  {#if folderSlot}
    {@const tag = slotScanning(folderSlot) ? "Scanning" : "Scanned"}
    {@const progress = scanProgress(folderSlot)}
    {@const active =
      shownSlot === FOLDER_SLOT ||
      (welcomeShown && app.target?.path === folderSlot.target.path)}
    <button
      class="side-nav disk-nav folder-nav scanned"
      class:active
      aria-current={active ? "page" : undefined}
      aria-label={`${folderSlot.target.name}, folder, ${tag}`}
      title={`${folderSlot.target.path}\n${tag}`}
      disabled={app.sessionBlocked}
      onclick={() => app.openDisk(folderSlot.target)}
    >
      <Icon name="folder" size={22} /><span class="nav-copy"
        ><span class="disk-name"
          ><strong>{folderSlot.target.name}</strong><em class:done={tag === "Scanned"}>{tag}</em></span
        ><small
          >{folderSlot.summary && !slotScanning(folderSlot)
            ? compactBytes(folderSlot.summary.allocatedBytes)
            : folderSlot.target.path}</small
        >{#if progress !== undefined}<span
            class="scan-track indeterminate"
            role="progressbar"
            aria-label="Scan progress"><span></span></span
          >{/if}</span
      >
    </button>
  {/if}
  {#if app.savedScansEnabled}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (The scroll region needs keyboard focus for Page Up/Down and Home/End.) -->
  <div
    class="sidebar-scroll"
    role="region"
    aria-label="Saved scans"
    tabindex="0"
    onscroll={() => (openMenu = null)}
  >
    <div
      class="side-label saved-label"
      title="Grouped by place · Newest first"
    >
      Saved Scans
    </div>
    <div class="saved-library">
      {#if app.saved.length === 0}<small class="empty-library"
          >No saved scans yet.</small
        >{/if}
      {#each savedGroups as group (group.rootPath)}
        {@const label = savedName(group.entries[0])}
        {@const expanded = expandedLocations.has(group.rootPath)}
        {@const visibleEntries = expanded
          ? group.entries
          : group.entries.filter(
              (entry, index) =>
                index === 0 || entry.id === app.summary?.savedId,
            )}
        {@const hiddenCount = group.entries.length - visibleEntries.length}
        {@const historyId = `saved-history-${encodeURIComponent(group.rootPath)}`}
        <section
          class="saved-group"
          data-root-path={group.rootPath}
          aria-label={`Saved scans for ${label}: ${group.rootPath}`}
        >
          <div class="saved-group-heading">
            <h3 title={group.rootPath}>
              <Icon
                name={group.rootPath === "/" ? "disk" : "folder"}
                size={15}
              />{label}
            </h3>
            <span
              >{group.entries.length}
              {group.entries.length === 1 ? "scan" : "scans"}</span
            >
          </div>
          <p class="saved-location-path" title={group.rootPath}>
            {group.rootPath}
          </p>
          <div id={historyId} class="saved-group-scans">
            {#each visibleEntries as entry (entry.id)}
              {@const scanned = formatScanTimestamp(
                entry.completedAtMs,
                undefined,
                group.duplicateSeconds.has(
                  Math.floor(entry.completedAtMs / 1000),
                ),
              )}
              {@const stored = formatScanTimestamp(
                entry.savedAtMs,
                undefined,
                group.duplicateCompletions.has(entry.completedAtMs),
              )}
              <div
                class="saved-entry"
                data-saved-id={entry.id}
                class:active={entry.id === openSavedId}
              >
                <button
                  class="saved-summary"
                  aria-current={entry.id === openSavedId ? "page" : undefined}
                  class:active={entry.id === openSavedId}
                  disabled={app.anyScanRunning || app.sessionBlocked}
                  aria-label={`Open saved scan ${label}, scanned ${scanned.absolute}, saved ${stored.absolute}`}
                  onclick={() => {
                    app.showQueue(false);
                    void app.openSaved(entry.id);
                  }}
                  title={`${entry.rootPath}\nScanned ${scanned.absolute}\nSaved ${stored.absolute}\n${entry.entries.toLocaleString()} entries · ${compactBytes(entry.allocatedBytes)} on disk${logicalSize.shown ? ` · ${compactBytes(entry.logicalBytes)} logical` : ""}`}
                >
                  <Icon name="clock" size={16} />
                  <span class="saved-copy">
                    <time class="saved-timestamp" datetime={scanned.iso}>
                      <span class="saved-day">{scanned.day}</span>
                      {#if scanned.time}<strong class="saved-clock"
                          >{scanned.time}</strong
                        >{/if}
                    </time>
                    <span class="saved-metadata">
                      <small
                        >{compactBytes(entry.allocatedBytes)} on disk · {entry.entries.toLocaleString(
                          undefined,
                          { notation: "compact", maximumFractionDigits: 1 },
                        )} entries</small
                      >
                      <span
                        class="saved-badge"
                        class:partial={!!coverageLabel(
                          app.summary?.savedId === entry.id
                            ? app.summary
                            : entry,
                        )}
                        >{coverageLabel(
                          app.summary?.savedId === entry.id
                            ? app.summary
                            : entry,
                        ) ?? "Saved"}</span
                      >
                    </span>
                    {#if group.duplicateCompletions.has(entry.completedAtMs)}
                      <small
                        class="saved-copy-time"
                        title={`Saved ${stored.absolute}`}
                        >Saved {stored.label}</small
                      >
                    {/if}
                  </span>
                </button>
                <div class="saved-actions">
                  <button
                    class="saved-menu-trigger"
                    aria-label={`Actions for saved scan ${label} (${scanned.absolute}; saved ${stored.absolute})`}
                    aria-expanded={openMenu === entry.id}
                    aria-controls={`saved-actions-${entry.id}`}
                    disabled={app.anyScanRunning || app.sessionBlocked}
                    onclick={(event) =>
                      toggleMenu(entry.id, event.currentTarget)}>⋯</button
                  >
                  {#if openMenu === entry.id}
                    <div
                      id={`saved-actions-${entry.id}`}
                      class="saved-menu"
                      style:top={`${menuPosition.top}px`}
                      style:left={`${menuPosition.left}px`}
                      use:dismissPopover={() => {
                        openMenu = null;
                      }}
                    >
                      {#if isTauri()}<button
                          aria-label={`Export saved scan ${label}`}
                          disabled={!app.can.exportSaved}
                          onclick={() => {
                            openMenu = null;
                            void app.exportScan(entry.id);
                          }}>Export scan</button
                        >{/if}
                      <button
                        aria-label={`Remove saved scan ${label}`}
                        disabled={app.anyScanRunning || app.sessionBlocked}
                        onclick={() => {
                          openMenu = null;
                          app.quickLook.dismiss();
                          app.setRemoveEntry(entry);
                        }}>Remove scan</button
                      >
                    </div>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
          {#if hiddenCount > 0 || (expanded && group.entries.length > 3)}
            <button
              class="saved-history-toggle"
              aria-expanded={expanded}
              aria-controls={historyId}
              aria-label={`${expanded ? "Show fewer scans" : `Show ${hiddenCount} older scans`} for ${label}: ${group.rootPath}`}
              disabled={app.anyScanRunning || app.sessionBlocked}
              onclick={() => toggleHistory(group.rootPath)}
              >{expanded
                ? "Show fewer"
                : `Show older scans (${hiddenCount})`}</button
            >
          {/if}
        </section>
      {/each}
    </div>
  </div>
  {/if}
  <div class="sidebar-actions">
    <button
      class="side-nav queue-nav"
      aria-label={`Cleanup queue · ${app.queue.entries.length}`}
      class:active={app.queueOpen}
      disabled={!app.can.cleanupQueue}
      onclick={() => app.showQueue(!app.queueOpen)}
    >
      <Icon name="queue" size={18} /><span>Cleanup queue</span><span
        class="queue-count">{app.queue.entries.length}</span
      ><Icon name="chevron" size={14} />
    </button>

    {#if app.savedScansEnabled}
    <button
      class="side-nav compare-button"
      aria-label="Compare saved scans"
      disabled={!app.can.compareScans}
      onclick={app.openComparison}
      ><Icon name="treemap" size={18} /><span>Compare scans</span></button
    >
    {#if isTauri()}<button
        class="side-nav import-button"
        aria-label="Import Scan"
        title="Shared scans include file names, full paths and sizes, but no file contents."
        disabled={!app.can.importScan}
        onclick={() => void app.importScan()}
        ><Icon name="import" size={18} /><span>Import scan</span></button
      >{/if}
    {/if}
  </div>
  <div class="side-bottom">
    <button
      class="settings-nav"
      aria-label={settingsLabel}
      disabled={app.anyScanRunning || app.sessionBlocked}
      onclick={() => app.openAccess()}
      ><Icon name="settings" size={19} /><span>Settings</span>{#if accessWarning}<i
          class="settings-dot"
          aria-hidden="true"
        ></i>{/if}</button
    >
  </div>
</aside>
