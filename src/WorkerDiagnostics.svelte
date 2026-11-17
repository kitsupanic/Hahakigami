<script module lang="ts">
  export function duration(us: number) {
    return us >= 1_000_000
      ? `${(us / 1_000_000).toFixed(2)} s`
      : `${(us / 1000).toFixed(1)} ms`;
  }
  export function entryShare(entries: number, totalEntries: number) {
    const total = Math.max(0, totalEntries - 1);
    return total ? (entries / total) * 100 : 0;
  }
</script>

<script lang="ts">
  import { qosLabel, type ScanSummary } from "./types";
  let {
    summary,
  }: {
    summary: ScanSummary;
  } = $props();
  let message = $state.raw("");
  let rows = $derived(summary.workerStats || []);
  let running = $derived(summary.state === "running");
  let shares = $derived(rows.map((row) => entryShare(row.entries, summary.entries)));
  let even = $derived(rows.length ? 100 / rows.length : 0);
  // One axis for every bar, with headroom so the busiest worker never touches the edge.
  let axis = $derived(Math.max(even, ...shares) * 1.25 || 1);
  const count = (value: number | null | undefined) =>
    value == null ? "—" : value.toLocaleString();
  // Workers run across and measures run down, so the table never needs 17 columns.
  let measures = $derived<[string, (row: (typeof rows)[number]) => string][]>([
    ["Folders", (row) => count(row.directories)],
    ["Relative opens", (row) => count(row.relativeOpens)],
    ["Path reopens", (row) => count(row.absoluteReopens)],
    ["Processing", (row) => duration(row.busyUs)],
    [
      "CPU",
      (row) =>
        row.cpuUs === null
          ? running
            ? "Pending"
            : "Unavailable"
          : duration(row.cpuUs),
    ],
    ["Metadata", (row) => duration(row.metadataUs)],
    ["Store wait", (row) => duration(row.storeWaitUs)],
    ["Commit", (row) => duration(row.commitUs)],
    ["Queue wait", (row) => duration(row.queueWaitUs)],
    ["Progress", (row) => duration(row.progressUs)],
    ["Lifetime", (row) => duration(row.elapsedUs)],
    ["Errors", (row) => String(row.errors)],
    ["Requested QoS", (row) => row.qos || "Pending"],
    ["QoS error", (row) => String(row.qosError ?? "—")],
  ]);
  async function copyReport() {
    try {
      if (!navigator.clipboard?.writeText)
        throw new Error("Clipboard is unavailable in this window.");
      await navigator.clipboard.writeText(
        JSON.stringify(
          {
            version: 1,
            scanId: summary.scanId,
            state: summary.state,
            workers: summary.workers,
            workerMode: summary.workerMode,
            workerSource: summary.workerSource,
            qos: summary.qos,
            elapsedMs: summary.elapsedMs,
            entries: summary.entries,
            errors: summary.errors,
            logicalBytes: summary.logicalBytes,
            allocatedBytes: summary.allocatedBytes,
            finalizationUs: summary.finalizationUs,
            descriptorLimit: summary.descriptorLimit,
            descriptorPeak: summary.descriptorPeak,
            workerStats: rows.map((row) => ({
              ...row,
              entrySharePercent: entryShare(row.entries, summary.entries),
            })),
            notes:
              "Entry share excludes the pre-created root. Processing includes metadata, store waits, commit and progress. CPU is sampled only at worker exit. Queue wait is dispatch/idle wait. Requested QoS is sampled at start and does not identify Performance/Efficiency core placement. UI/query work and final aggregation are outside worker CPU totals.",
          },
          null,
          2,
        ),
      );
      message = "Worker report copied.";
    } catch (e) {
      message = String(e);
    }
  }
</script>

<div class="worker-diagnostics">
  <div class="diagnostics-heading">
    <p>
      {#if running}Live counters. CPU time is available when each worker exits.{/if}
    </p>
    <button disabled={!rows.length} onclick={() => void copyReport()}
      >Copy worker report</button
    >
  </div>
  <dl class="diagnostics-facts">
    <div>
      <dt>Workers</dt>
      <dd>{summary.workers}{summary.qos ? ` · ${qosLabel(summary.qos)}` : ""}</dd>
    </div>
    {#if summary.descriptorLimit != null}
      <div>
        <dt>Directory handles</dt>
        <dd>
          {summary.descriptorPeak ?? "pending"} peak / {summary.descriptorLimit} limit
        </dd>
      </div>
    {/if}
    <div>
      <dt>Final aggregation</dt>
      <dd>{duration(summary.finalizationUs || 0)}</dd>
    </div>
  </dl>
  {#if rows.some((row) => row.qosError != null)}<p role="alert">
      Some workers could not apply the scheduling preference. They continued
      with their existing priority; see the QoS error row below.
    </p>{/if}
  {#if rows.length}
    <table class="share-bars">
      <thead
        ><tr
          ><th scope="col">Worker</th><th scope="col" aria-hidden="true">Share of entries</th><th
            scope="col">Entries</th
          ><th scope="col">Share</th></tr
        ></thead
      >
      <tbody>
        {#each rows as row, index}
          <tr
            ><th scope="row">{row.id}</th><td class="share-track" aria-hidden="true"
              ><span
                ><i style:width={`${(shares[index] / axis) * 100}%`}></i><b
                  style:left={`${(even / axis) * 100}%`}
                ></b></span
              ></td
            ><td>{row.entries.toLocaleString()}</td><td
              >{shares[index].toFixed(1)}%</td
            ></tr
          >
        {/each}
      </tbody>
    </table>
    <p class="share-key">The notch marks an even share, {even.toFixed(1)}%.</p>
    <div class="diagnostics-table-wrap">
      <table class="measures">
        <thead
          ><tr
            ><th scope="col">Worker</th>{#each rows as row}<th scope="col"
                >{row.id}</th
              >{/each}</tr
          ></thead
        >
        <tbody>
          {#each measures as [label, value]}
            <tr
              ><th scope="row">{label}</th>{#each rows as row}<td
                  >{value(row)}</td
                >{/each}</tr
            >
          {/each}
        </tbody>
      </table>
    </div>
  {:else}<p>Worker data appears as the scan starts.</p>{/if}
  {#if message}<p role="status">{message}</p>{/if}
</div>

<style>
  .worker-diagnostics {
    font-size: var(--font-secondary);
  }
  p {
    max-width: 68ch;
    margin: 8px 0;
    font-size: var(--font-caption);
    line-height: 1.5;
    color: var(--muted);
  }
  .diagnostics-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .diagnostics-heading button {
    white-space: nowrap;
    font-size: var(--font-caption);
  }
  .diagnostics-facts {
    display: flex;
    flex-wrap: wrap;
    margin: 4px 0 14px;
    border: 1px solid var(--control-line, var(--line));
    background: var(--control-surface);
  }
  .diagnostics-facts > div {
    flex: 1 1 auto;
    padding: 6px 10px;
    box-shadow: 1px 0 0 var(--control-line, var(--line));
  }
  .diagnostics-facts dt {
    font-size: var(--font-caption);
    color: var(--muted);
  }
  .diagnostics-facts dd {
    margin: 0;
    font-family: var(--font-figures);
    color: var(--text);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--font-figures);
    font-size: var(--font-caption);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  th,
  td {
    padding: 4px 8px;
    text-align: right;
    white-space: nowrap;
  }
  th {
    font-family: var(--font-sans);
    font-weight: 600;
    color: var(--muted);
  }
  th:first-child,
  .share-bars th:nth-child(2) {
    text-align: left;
  }
  .share-bars th:first-child,
  .share-bars td:first-child {
    width: 1%;
    padding-left: 0;
  }
  .share-bars td:nth-child(n + 3),
  .share-bars th:nth-child(n + 3) {
    width: 1%;
  }
  .share-bars td:last-child,
  .share-bars th:last-child {
    padding-right: 0;
  }
  .share-track {
    padding-block: 0;
  }
  /* Bar and notch share one box, so a share and the even share line up exactly. */
  .share-track > span {
    position: relative;
    display: block;
  }
  .share-track i {
    display: block;
    height: 12px;
    min-width: 1px;
    background: var(--text);
  }
  /* The even-share notch: a panel-coloured gap with a hairline through it. */
  .share-track b {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--panel);
    outline: 1px solid var(--muted);
  }
  .diagnostics-table-wrap {
    overflow-x: auto;
    border: 1px solid var(--control-line, var(--line));
    background: var(--control-surface);
  }
  .measures th,
  .measures td {
    padding: 5px 8px;
    border-top: 1px solid var(--line);
  }
  .measures thead th {
    border-top: 0;
    border-bottom: 1px solid var(--control-line, var(--line));
    color: var(--text);
  }
  /* The measure names stay put while many workers scroll sideways. */
  .measures th:first-child {
    position: sticky;
    left: 0;
    background: var(--control-surface);
  }
</style>
