<script lang="ts">
  import Icon from "./Icon.svelte";
  import { formatScanTimestamp } from "./savedHistory";
  import { compactBytes } from "./uiFormat";
  import type { ScanTransferReport } from "./scanTransferTypes";

  // Pending transfers only. Failures open in FailureModal.
  let { report }: { report: ScanTransferReport } = $props();
  const importing = $derived(report.operation === "import");
  const title = $derived(importing ? "Importing scan…" : "Exporting scan…");
  const phase = $derived(
    report.phase === "choosing"
      ? importing
        ? "Choose a shared scan file in the file dialog."
        : "Choose a destination in the save dialog."
      : report.phase === "processing"
        ? importing
          ? "Checking the archive and adding it to Saved Scans…"
          : "Preparing and writing the shared scan…"
        : `Finishing ${report.operation}…`,
  );
</script>

<section
  class="scan-transfer"
  aria-label="Saved scan transfer"
  data-operation={report.operation}
  data-state={report.state}
  data-phase={report.phase}
>
  <div class="transfer-heading">
    <span class="transfer-icon"><Icon name={report.operation} size={18} /></span>
    <div role="status" aria-live="polite" aria-atomic="true">
      <h2>{title}</h2>
      <p class="transfer-phase">{phase}</p>
    </div>
  </div>
  {#if report.phase !== "choosing"}<div
      class="transfer-activity scan-activity"
      aria-hidden="true"
    >
      <span></span>
    </div>{/if}
  {#if report.phase === "processing" && report.bytes > 0}
    <p
      class="transfer-bytes"
      title="Scan data handled so far. This can start over between reading and writing."
    >
      Scan data processed <strong>{compactBytes(report.bytes)}</strong>
    </p>
  {/if}
  {#if report.entry}
    {@const timestamp = formatScanTimestamp(
      report.entry.completedAtMs,
      undefined,
      true,
    )}
    <div class="transfer-identity">
      <span>Scanned location</span>
      <strong
        >{report.entry.rootPath === "/"
          ? "Startup disk"
          : report.entry.name}</strong
      >
      <span class="transfer-path">{report.entry.rootPath}</span>
      <time
        class="transfer-timestamp"
        datetime={timestamp.iso}
        title={timestamp.absolute}>Scanned {timestamp.absolute}</time
      >
    </div>
  {/if}
  {#if !importing}<p class="transfer-note">
      Shared scans include file names, full paths and sizes, but no file contents.
    </p>{/if}
</section>
