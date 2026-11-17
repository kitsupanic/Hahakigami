<script lang="ts">
  import ScanIssues from "./ScanIssues.svelte";
  import WorkerDiagnostics from "./WorkerDiagnostics.svelte";
  import { coverageLabel } from "./scanCoverage";
  import { dismissPopover } from "./dialogFocus";
  import { bytes, type Metric, type ScanSummary } from "./types";
  import { compactBytes, scanTime } from "./uiFormat";
  import { logicalSize } from "./logicalSize.svelte";
  import type { Snippet } from "svelte";

  // The status strip's one details popover. The totals cells open it on Summary and the
  // Coverage cell opens it on Coverage; ScanStatusLine keys it by scan so a new scan starts closed.
  type Tab = "summary" | "coverage" | "workers";
  type Opener = "totals" | "coverage";
  let {
    summary,
    metric,
    historical,
    description,
    lead,
  }: {
    summary: ScanSummary;
    metric: Metric;
    historical: boolean;
    description: string;
    /** The strip's Scanned and Time cells, which open Summary like the totals. */
    lead?: Snippet;
  } = $props();

  let restricted = $derived(
    Math.min(summary.errors, Math.max(0, summary.restrictedSystemIssues ?? 0)),
  );
  let other = $derived(Math.max(0, summary.errors - restricted));
  // Restricted system locations and excluded mounts are expected, so only real problems surface here.
  let hasCoverage = $derived(!!coverageLabel(summary));
  let soFar = $derived(summary.state === "running" ? " so far" : "");
  let entries = $derived(
    new Intl.NumberFormat(undefined, {
      notation: "compact",
      maximumFractionDigits: 1,
    }).format(summary.entries),
  );
  let countDescription = $derived(
    [
      other
        ? `${other.toLocaleString()} other scan issue ${other === 1 ? "event" : "events"}`
        : "",
    ]
      .filter(Boolean)
      .join(" · ") ||
      "Incomplete coverage; no detailed issue events were recorded.",
  );

  let tab: Tab | null = $state.raw(null);
  let openedBy: Opener | null = $state.raw(null);
  let id = $derived(`scan-details-${encodeURIComponent(summary.scanId)}`);
  let tabs = $derived<Tab[]>(
    hasCoverage ? ["summary", "coverage", "workers"] : ["summary", "workers"],
  );
  const tabLabels: Record<Tab, string> = {
    summary: "Summary",
    coverage: "Coverage",
    workers: "Workers",
  };

  function toggle(by: Opener, start: Tab) {
    if (openedBy === by) {
      tab = openedBy = null;
    } else {
      tab = start;
      openedBy = by;
    }
  }
  function close() {
    tab = openedBy = null;
  }
  function moveTab(event: KeyboardEvent) {
    const step =
      event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    const index =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? tabs.length - 1
          : step
            ? (tabs.indexOf(tab!) + step + tabs.length) % tabs.length
            : -1;
    if (index < 0) return;
    event.preventDefault();
    tab = tabs[index];
    document.getElementById(`${id}-tab-${tab}`)?.focus();
  }
</script>

<div class="scan-details">
  <button
    class="scan-totals"
    aria-expanded={openedBy === "totals"}
    aria-controls={id}
    title={`${summary.entries.toLocaleString()} entries · scanned on disk`}
    onclick={() => toggle("totals", "summary")}
    >{@render lead?.()}<span class="status-cell"
      ><small>On disk</small><span class="status-value is-size"
        >{compactBytes(summary.allocatedBytes)}</span
      ></span
    ><span class="status-cell"
      ><small>Entries</small><span class="status-value is-count">{entries}</span
      ></span
    ></button
  >
  <div
    class="scan-details-popover"
    {id}
    hidden={!tab}
    use:dismissPopover={close}
  >
    <div class="scan-details-tabs" role="tablist" aria-label="Scan details">
      {#each tabs as name}
        <button
          role="tab"
          id={`${id}-tab-${name}`}
          aria-selected={tab === name}
          aria-controls={`${id}-${name}`}
          tabindex={tab === name ? 0 : -1}
          onclick={() => (tab = name)}
          onkeydown={moveTab}
          >{tabLabels[name]}{#if name === "coverage"}<em
              >{other.toLocaleString()}</em
            >{:else if name === "workers"}<em class="scan-workers"
              >{summary.workers}</em
            >{/if}</button
        >
      {/each}
    </div>
    <div
      class="scan-details-panel scan-information"
      role="tabpanel"
      id={`${id}-summary`}
      aria-labelledby={`${id}-tab-summary`}
      tabindex="0"
      hidden={tab !== "summary"}
    >
      {#if summary.state !== "completed"}<p class="scan-information-description">
          {description}
        </p>{/if}
      <dl class="scan-facts">
        <div class:active={metric === "allocated"}>
          <dt>On disk{soFar}</dt>
          <dd title={`${summary.allocatedBytes.toLocaleString()} bytes`}>
            {bytes(summary.allocatedBytes)}
          </dd>
        </div>
        {#if logicalSize.shown}
          <div class:active={metric === "logical"}>
            <dt>Logical size{soFar}</dt>
            <dd title={`${summary.logicalBytes.toLocaleString()} bytes`}>
              {bytes(summary.logicalBytes)}
            </dd>
          </div>
        {/if}
        <div>
          <dt>Entries{soFar}</dt>
          <dd>{summary.entries.toLocaleString()}</dd>
        </div>
        <div class="storage-duration">
          <dt
            title="The timer may pause while checking the last scan or saving data for the next one."
          >
            Scan duration
          </dt>
          <dd title={`${summary.elapsedMs.toLocaleString()} ms`}>
            {scanTime(summary.elapsedMs)}
          </dd>
        </div>
        {#if !historical && summary.scanMode}
          <div>
            <dt>Scan type</dt>
            <dd>
              {summary.scanMode === "incremental"
                ? `Incremental · ${(summary.reusedEntries || 0).toLocaleString()} metadata entries reused`
                : "Full scan"}
            </dd>
          </div>
        {/if}
      </dl>
      {#if summary.message}<p class="scan-message">{summary.message}</p>{/if}
      {#if summary.accountingWarnings?.length}<p class="scan-message">
          Some files changed during the scan, so a few sizes may be slightly off.
        </p>{/if}
      {#if !historical && summary.cacheWarning}
        <p class="scan-note">{summary.cacheWarning}</p>
      {/if}
    </div>
    {#if hasCoverage}
      <div
        class="scan-details-panel"
        role="tabpanel"
        id={`${id}-coverage`}
        aria-labelledby={`${id}-tab-coverage`}
        hidden={tab !== "coverage"}
      >
        <ScanIssues {summary} active={tab === "coverage"} />
      </div>
    {/if}
    <div
      class="scan-details-panel"
      role="tabpanel"
      id={`${id}-workers`}
      aria-labelledby={`${id}-tab-workers`}
      hidden={tab !== "workers"}
    >
      <WorkerDiagnostics {summary} />
    </div>
  </div>
  {#if hasCoverage}
    <div class="scan-status-row">
    <div class="scan-status-summary sr-only" id={`${id}-counts`}>
      <strong class="scan-coverage-label">{coverageLabel(summary)}</strong>
      {#if other > 0}<span
          aria-label={`${other.toLocaleString()} other scan issue ${other === 1 ? "event" : "events"}`}
          >{other.toLocaleString()} issue {other === 1 ? "event" : "events"}</span
        >{/if}
    </div>
    <button
      class="scan-coverage-chip"
      class:partial-coverage={true}
      aria-expanded={openedBy === "coverage"}
      aria-controls={id}
      aria-describedby={`${id}-counts`}
      title={countDescription}
      aria-label={openedBy === "coverage"
        ? "Hide scan details"
        : "Show scan details"}
      onclick={() => toggle("coverage", "coverage")}
      ><small>Coverage</small><span class="scan-coverage-value"
        >Limited<span aria-hidden="true">{" · "}</span><u
          >{openedBy === "coverage" ? "Hide details" : "Details"}</u
        ></span
      ></button
    >
    </div>
  {/if}
</div>

<style>
  /* No box of its own: the triggers stay cells of the status strip, and the popover
     anchors to the strip. The wrapper only scopes light-dismiss to these triggers. */
  .scan-details {
    display: contents;
  }
  .scan-totals {
    display: flex;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    white-space: nowrap;
  }
  .scan-totals:hover :global(.status-cell),
  .scan-totals[aria-expanded="true"] :global(.status-cell) {
    background: var(--soft);
  }
  .scan-status-row {
    display: flex;
    align-items: center;
  }
  .scan-coverage-chip {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 1px;
    min-height: 0;
    padding: 4px 12px;
    border: 0;
    border-right: 1px solid var(--line);
    border-radius: 0;
    line-height: 1.25;
    background: transparent;
    color: var(--text);
    font-size: var(--font-caption);
    text-align: left;
    white-space: nowrap;
  }
  .scan-coverage-chip:hover,
  .scan-coverage-chip[aria-expanded="true"] {
    background: var(--soft);
  }
  .scan-coverage-chip > small {
    font-size: var(--font-caption);
    color: var(--muted);
  }
  .scan-coverage-value {
    font-size: var(--font-secondary);
  }
  .scan-coverage-value u {
    text-underline-offset: 2px;
    color: var(--muted);
  }
  /* Dashed means incomplete, solid means complete (the website's design system). Not a warning colour:
     restricted coverage is the expected result without Full Disk Access. */
  .scan-coverage-chip.partial-coverage {
    margin: 3px 0 3px 6px;
    padding-block: 2px;
    border: 1px dashed var(--muted);
  }
  .scan-details-popover {
    position: absolute;
    top: calc(100% + 8px);
    left: 12px;
    z-index: 15;
    width: min(640px, calc(100% - 24px));
    max-height: min(70vh, calc(100dvh - 160px));
    overflow: auto;
    padding: 0 16px 14px;
    border: 1px solid var(--line);
    border-radius: var(--radius-md);
    background: var(--panel);
    box-shadow: var(--shadow-popover);
    color: var(--muted);
    font-size: var(--font-secondary);
  }
  /* The tabs stay in view while a long issue list or worker table scrolls under them. */
  .scan-details-tabs {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    gap: 2px;
    margin: 0 -16px 14px;
    padding: 10px 16px 0;
    border-bottom: 1px solid var(--control-line, var(--line));
    background: var(--panel);
  }
  .scan-details-tabs button {
    min-height: 0;
    margin-bottom: -1px;
    padding: 6px 12px 5px;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    background: transparent;
    color: var(--muted);
    font-size: var(--font-secondary);
    font-weight: 600;
  }
  .scan-details-tabs button:hover {
    color: var(--text);
  }
  .scan-details-tabs button[aria-selected="true"] {
    border-color: var(--control-line, var(--line));
    background: var(--panel);
    color: var(--text);
  }
  .scan-details-tabs em {
    margin-left: 5px;
    font-family: var(--font-figures);
    font-style: normal;
    font-weight: 500;
    color: var(--muted);
  }
  .scan-information-description {
    max-width: 62ch;
    margin: 0 0 10px;
    line-height: 1.5;
  }
  .scan-facts {
    margin: 0 0 8px;
  }
  .scan-facts > div {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 16px;
    padding: 7px 0;
    border-top: 1px solid var(--line);
  }
  .scan-facts dd {
    margin: 0;
    font-family: var(--font-figures);
    font-variant-numeric: tabular-nums;
    text-align: right;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .scan-facts .active dt {
    font-weight: 600;
    color: var(--text);
  }
  .scan-message {
    margin: 3px 0;
    font-size: var(--font-caption);
    overflow-wrap: anywhere;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
