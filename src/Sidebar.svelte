<script lang="ts">
  import type createApp from "./createApp.svelte";
  import Icon from "./Icon.svelte";
  import appIcon from "../src-tauri/icons/128x128.png";
  import { compactBytes } from "./uiFormat";
  import { FOLDER_SLOT, slotScanning, type ScanSlot } from "./scanSlots.svelte";
  import "./sidebar.css";
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
  const explorerShown = $derived(!app.homeOpen && !app.queueOpen);
  const welcomeShown = $derived(
    !app.queueOpen && (app.homeOpen || !app.summary),
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
</script>

<aside
  class="sidebar compact-sidebar"
  id="locations-sidebar"
  hidden={!visible}
  inert={inert ||
    app.accessOpen ||
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
