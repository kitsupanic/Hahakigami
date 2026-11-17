<script lang="ts">
  import { coverageLabel } from "./scanCoverage";
  import ScanDetails from "./ScanDetails.svelte";
  import type { Metric, ScanSummary } from "./types";
  import { scanTime } from "./uiFormat";
  import { formatScanTimestamp } from "./savedHistory";
  import type { Snippet } from "svelte";

  let {
    summary,
    metric,
    historical,
    cancelling,
    canSave,
    savesEnabled = true,
    liveSaved = false,
    savedTimestamp,
    savedTimestampPrecise = false,
    onSave,
    controls,
  }: {
    summary: ScanSummary;
    metric: Metric;
    historical: boolean;
    cancelling: boolean;
    canSave: boolean;
    /** False when saved scans are turned off for this build: no Save chip at all. */
    savesEnabled?: boolean;
    liveSaved?: boolean;
    savedTimestamp?: number;
    savedTimestampPrecise?: boolean;
    onSave: () => void;
    /** The map's view controls, kept on this row so the strip is the one toolbar under the header. */
    controls?: Snippet;
  } = $props();
  let running = $derived(summary.state === "running");
  let completed = $derived(summary.state === "completed");
  let restricted = $derived(
    Math.min(summary.errors, Math.max(0, summary.restrictedSystemIssues ?? 0)),
  );
  let otherIssues = $derived(Math.max(0, summary.errors - restricted));
  let partial = $derived(!!coverageLabel(summary));
  let status = $derived(
    running
      ? cancelling
        ? "Stopping scan…"
        : summary.scanPhase === "checking"
          ? "Checking previous scan…"
          : summary.scanPhase === "verifying"
            ? "Verifying shared clone sizes…"
            : summary.scanPhase === "caching"
              ? "Finishing scan…"
              : "Scanning…"
      : completed
        ? otherIssues > 0
          ? "Scan complete · scan issues"
          : partial
            ? "Scan complete · limited coverage"
            : "Scan complete"
        : summary.state === "cancelled"
          ? "Scan cancelled · partial results"
          : "Scan failed",
  );
  let announced = $derived(liveSaved ? `${status} · Scan saved` : status);
  let stateLabel = $derived(
    running
      ? "Scanning"
      : completed
        ? "Scanned"
        : summary.state === "cancelled"
          ? "Cancelled"
          : "Scan failed",
  );
  let phaseLabel = $derived(
    cancelling
      ? "Stopping…"
      : summary.scanPhase === "checking"
        ? "Checking cache…"
        : summary.scanPhase === "verifying"
          ? "Verifying clones…"
          : summary.scanPhase === "caching"
            ? "Finishing…"
            : "Counting files…",
  );
  let description = $derived(
    running
      ? cancelling
        ? "Waiting for the current filesystem operation to stop. Partial results will remain available."
        : summary.scanPhase === "checking"
          ? "Validating previous scan metadata. Changes may require a full scan."
          : summary.scanPhase === "verifying"
            ? "Every file and folder has been found and can be explored. Sizes of cloned files may shrink as shared storage is confirmed."
            : summary.scanPhase === "caching"
              ? "Keeping metadata for faster rescans. Finalizing the scan results."
              : "Finding files and folders. Counts and sizes update as the scan progresses."
      : completed
        ? partial
          ? "Results are ready to explore, with incomplete coverage. Some data could not be scanned."
          : "Results are ready to explore. Select a folder or file to see what uses your space."
        : summary.state === "cancelled"
          ? "Only the entries found before stopping are shown. Scan again for complete results."
          : "The scan could not finish. Any results shown are partial. Review the error and try scanning again.",
  );
  let timestamp = $derived(
    historical
      ? savedTimestamp
      : (summary.completedAtMs ?? summary.startedAtMs),
  );
</script>

<div
  class="scan-status-line"
  data-state={summary.state}
  data-phase={summary.scanPhase}
>
  <!-- The visible state lives in the first cell; this region only announces it. -->
  <span
    class="scan-status-text sr-only"
    role={historical ? undefined : "status"}
    aria-live={historical ? undefined : "polite"}
    aria-atomic={historical ? undefined : "true"}
    title={status}>{announced}</span
  >
  {#key summary.scanId}
    <ScanDetails {summary} {metric} {historical} {description}>
      {#snippet lead()}
      <span
        class="scan-timestamp status-cell"
        title={running || timestamp == null
          ? status
          : `Scan ${historical || summary.completedAtMs != null ? "completed" : "started"} ${formatScanTimestamp(timestamp).absolute}`}
        ><small>{stateLabel}</small>
        <span class="status-value is-state"
          >{running
            ? phaseLabel
            : timestamp == null
              ? "—"
              : formatScanTimestamp(timestamp, undefined, savedTimestampPrecise)
                  .label}</span
        >{#if running}<span class="scan-progress" aria-hidden="true"></span>{/if}</span
      >
      <span class="scan-elapsed status-cell" title="Scan duration"
        ><small>Time</small><span class="status-value is-time"
          >{scanTime(summary.elapsedMs)}</span
        ></span
      >
      {/snippet}
    </ScanDetails>
  {/key}
  {#if !historical && summary.cacheWarning}
    <span class="scan-cache-note" title={summary.cacheWarning}
      >Rescan cache unavailable</span
    >
  {/if}
  {#if summary.state === "failed" || summary.state === "cancelled"}
    <span class="scan-state-description">{description}</span>
  {/if}
  <div class="scan-status-end">
    {@render controls?.()}
    {#if historical}
      <span class="status-chip is-saved">Saved scan</span>
    {:else if completed && savesEnabled}
      <!-- One button for both states so keyboard focus survives saving. -->
      <button
        class="status-chip"
        class:is-unsaved={!liveSaved}
        class:is-saved={liveSaved}
        aria-disabled={liveSaved || !canSave}
        onclick={() => !liveSaved && canSave && onSave()}
        >{liveSaved ? "Saved" : "Save scan"}</button
      >
    {/if}
  </div>
</div>

<style>
  /* A spec label: ruled cells of small-print facts, the one action at the end. */
  .scan-status-line {
    position: relative;
    z-index: 8;
    order: -2;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 0;
    min-width: 0;
    min-height: 40px;
    padding: 0 20px 0 0;
    border-bottom: 1px solid var(--line);
    font-size: var(--font-caption);
    color: var(--muted);
  }
  .scan-cache-note,
  .scan-state-description {
    padding: 4px 12px;
  }
  .scan-status-line :global(.status-cell) {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 1px;
    padding: 4px 12px;
    border-right: 1px solid var(--line);
    line-height: 1.25;
    white-space: nowrap;
  }
  .scan-status-line :global(.status-cell > small) {
    font-size: var(--font-caption);
    color: var(--muted);
  }
  .scan-status-line :global(.status-value) {
    font-family: var(--font-figures);
    font-size: var(--font-secondary);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  /* Room for the widest value each cell takes, so cells keep still while a scan counts up. */
  /* Fixed so the strip never shifts; older dates truncate, the title keeps the full one. */
  .status-value.is-state {
    width: 19ch;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .scan-timestamp {
    position: relative;
    overflow: hidden;
    padding-inline: 20px;
  }
  [data-state="cancelled"] .scan-timestamp > small {
    color: var(--warning);
  }
  [data-state="failed"] .scan-timestamp > small {
    color: var(--danger);
  }
  /* An indeterminate rule that sweeps between the cell's text edges while the scan runs. */
  .scan-progress {
    position: absolute;
    bottom: 0;
    left: 20px;
    width: 30%;
    height: 2px;
    background: var(--accent);
    animation: scan-progress 1.2s ease-in-out infinite alternate;
  }
  @keyframes scan-progress {
    to {
      left: calc(70% - 20px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .scan-progress {
      width: calc(100% - 40px);
      animation: none;
      opacity: 0.6;
    }
  }
  .status-value.is-time {
    min-width: 12ch;
  }
  .scan-status-line :global(.status-value.is-size) {
    min-width: 10ch;
  }
  .scan-status-line :global(.status-value.is-count) {
    min-width: 6ch;
  }
  .scan-status-end {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
    padding-left: 12px;
  }
  .status-chip {
    min-height: 0;
    padding: 3px 10px;
    border: 1px solid var(--line);
    border-radius: var(--radius-sm);
    font-size: var(--font-secondary);
    font-weight: 600;
    white-space: nowrap;
  }
  .status-chip[aria-disabled="true"] {
    cursor: default;
  }
  /* Saving is the strip's one action, so it takes the accent fill. */
  .status-chip.is-unsaved {
    color: var(--on-accent);
    background: var(--accent);
    border-color: var(--accent);
  }
  .status-chip.is-unsaved:not([aria-disabled="true"]):hover {
    background: var(--accent-hover);
  }
  .status-chip.is-unsaved[aria-disabled="true"] {
    opacity: 0.55;
  }
  .status-chip.is-saved {
    color: var(--success);
    background: transparent;
    border-color: var(--success-line);
  }
  .scan-state-description {
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
