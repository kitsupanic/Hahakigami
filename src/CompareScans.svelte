<script module lang="ts">
  import type { Comparison } from "./comparisonTypes";
  import type { LibraryProgress, SavedEntry } from "./types";
  export interface CompareScansProps {
    entries: SavedEntry[];
    /** The finished Current scan, comparable without saving it. */
    current?: { id: string; rootPath: string; completedAtMs: number } | null;
    disabled: boolean;
    result: Comparison | null;
    progress: LibraryProgress | null;
    onCompare: (beforeId: string, afterId: string) => void;
    onClose: () => void;
  }
</script>

<script lang="ts">
  import type { ComparisonRanking, ComparisonTotals } from "./comparisonTypes";
  import { bytes } from "./types";
  import { compactBytes } from "./uiFormat";
  import { logicalSize } from "./logicalSize.svelte";
  import { formatScanTimestamp, groupSavedScans } from "./savedHistory";
  import "./comparison.css";
  const signed = (value: number, full = false) =>
    `${value > 0 ? "+" : value < 0 ? "−" : ""}${full ? bytes(Math.abs(value), 1) : compactBytes(Math.abs(value), 1)}`;
  const trend = (value: number) =>
    value > 0 ? "growth" : value < 0 ? "shrinkage" : "unchanged";
  const phaseLabels: Record<string, string> = {
    readingBefore: "Opening before scan",
    readingAfter: "Opening after scan",
    comparing: "Comparing saved results",
    done: "Comparison complete",
  };
  const views = [
    {
      direction: "growth",
      label: "Grew",
      heading: "What grew",
      copy: "Entries with a larger recorded size, including new entries with a positive size.",
    },
    {
      direction: "shrinkage",
      label: "Shrank",
      heading: "What shrank",
      copy: "Entries with a smaller recorded size, including removed entries with a negative size change.",
    },
    {
      direction: "added",
      label: "Added",
      heading: "Added entries",
      copy: "Entries recorded only in the After scan, including zero-byte entries.",
    },
    {
      direction: "removed",
      label: "Removed",
      heading: "Removed entries",
      copy: "Entries recorded only in the Before scan, including zero-byte entries.",
    },
    {
      direction: "changed",
      label: "Changed",
      heading: "Changed entries",
      copy: "Entries whose recorded size or other metadata changed. File contents and modification times are not compared; a zero size change can still have changed metadata.",
    },
  ] as const;
  type Choice = Pick<
    SavedEntry,
    "id" | "name" | "rootPath" | "completedAtMs" | "savedAtMs"
  >;
  function defaultPair(entries: Choice[]) {
    for (const after of entries) {
      const before = entries.find(
        (entry) => entry.id !== after.id && entry.rootPath === after.rootPath,
      );
      if (before) return { beforeId: before.id, afterId: after.id };
    }
    return { beforeId: "", afterId: "" };
  }
  let {
    entries,
    current = null,
    disabled,
    result,
    progress,
    onCompare,
    onClose,
  }: CompareScansProps = $props();
  let ordered = $derived(
    [...entries].sort(
      (a, b) => b.completedAtMs - a.completedAtMs || b.savedAtMs - a.savedAtMs,
    ),
  );
  let currentChoice: Choice | null = $derived(
    current && {
      ...current,
      name: "Current scan",
      savedAtMs: current.completedAtMs,
    },
  );
  let choices: Choice[] = $derived(
    currentChoice ? [currentChoice, ...ordered] : ordered,
  );
  // Against the Current scan, offer only saved scans of the same location.
  function optionsFor(other: Choice | undefined) {
    if (!currentChoice) return ordered;
    if (other?.id === currentChoice.id)
      return ordered.filter((entry) => entry.rootPath === currentChoice!.rootPath);
    return other && other.rootPath !== currentChoice.rootPath ? ordered : choices;
  }
  let groups = $derived(groupSavedScans(entries));
  function timestamps(entry: Choice) {
    const group =
      entry.id === currentChoice?.id
        ? undefined
        : groups.find((group) => group.rootPath === entry.rootPath);
    const duplicate = !!group?.duplicateCompletions.has(entry.completedAtMs);
    return {
      scanned: formatScanTimestamp(
        entry.completedAtMs,
        undefined,
        group?.duplicateSeconds.has(Math.floor(entry.completedAtMs / 1000)),
      ),
      stored: formatScanTimestamp(entry.savedAtMs, undefined, duplicate),
      duplicate,
      copy:
        group?.entries
          .filter((item) => item.completedAtMs === entry.completedAtMs)
          .findIndex((item) => item.id === entry.id) ?? 0,
    };
  }
  function scanLabel(entry: Choice) {
    if (entry.id === currentChoice?.id)
      return `Current scan · ${entry.rootPath}`;
    const time = timestamps(entry);
    return `${entry.name} · ${time.scanned.label}${time.duplicate ? ` · saved ${time.stored.label} · saved result ${time.copy + 1}` : ""} · ${entry.rootPath}`;
  }
  let selection: { beforeId: string; afterId: string } | null =
    $state.raw(null);
  let chosenMetric: ComparisonRanking["metric"] = $state.raw("allocated");
  const metric = $derived(logicalSize.shown ? chosenMetric : "allocated");
  const sizeMetrics = $derived(
    logicalSize.shown ? (["allocated", "logical"] as const) : (["allocated"] as const),
  );
  let kind: ComparisonRanking["kind"] = $state.raw("files");
  let direction: ComparisonRanking["direction"] = $state.raw("growth");
  let selected = $derived.by(() => {
    if (selection) return selection;
    const pair = defaultPair(ordered);
    return pair.afterId || !currentChoice
      ? pair
      : {
          beforeId:
            ordered.find((e) => e.rootPath === currentChoice.rootPath)?.id ??
            "",
          afterId: currentChoice.id,
        };
  });
  let before = $derived(
    choices.find((entry) => entry.id === selected.beforeId),
  );
  let after = $derived(choices.find((entry) => entry.id === selected.afterId));
  let withCurrent = $derived(
    !!currentChoice &&
      (before?.id === currentChoice.id || after?.id === currentChoice.id),
  );
  let busy = $derived(disabled || (!!progress && progress.phase !== "done"));
  let compatible = $derived(
    !!before &&
      !!after &&
      before.id !== after.id &&
      before.rootPath === after.rootPath,
  );
  let currentResult = $derived(
    compatible &&
      result &&
      result.beforeId === before?.id &&
      result.afterId === after?.id &&
      result.rootPath === before?.rootPath
      ? result
      : null,
  );
  let ranking = $derived(
    currentResult?.rankings.find(
      (item) =>
        item.metric === metric &&
        item.kind === kind &&
        item.direction === direction,
    ),
  );
  let rows = $derived(ranking?.items.slice(0, 100) ?? []);
  let total = $derived(ranking?.total ?? 0);
  let activeView = $derived(
    views.find((view) => view.direction === direction)!,
  );
  let net = $derived(
    currentResult
      ? metric === "allocated"
        ? currentResult.allocatedDelta
        : currentResult.logicalDelta
      : 0,
  );
  const metricSize = (totals: ComparisonTotals) =>
    metric === "allocated" ? totals.allocatedBytes : totals.logicalBytes;
  const incomplete = (totals: ComparisonTotals) =>
    totals.incomplete || totals.errors > 0;
  const coverage = (totals: ComparisonTotals) =>
    incomplete(totals)
      ? "Incomplete coverage"
      : totals.excluded > 0
        ? "Has exclusions"
        : "Complete coverage";
  function pathParts(path: string) {
    const split = path.lastIndexOf("/");
    return {
      name: path.slice(split + 1) || path,
      parent: split > 0 ? path.slice(0, split) : split === 0 ? "/" : "",
    };
  }
</script>

<section
  class="comparison-panel"
  aria-labelledby="comparison-title"
  aria-busy={busy}
>
  <div class="comparison-heading">
    <div>
      <h2 id="comparison-title">Compare saved scans</h2>
      <p>
        {withCurrent
          ? "Comparing the Current scan with a saved scan"
          : "Comparing two past scans"} · Read-only
      </p>
    </div>
    <button disabled={busy} onclick={onClose}>Close comparison</button>
  </div>
  <div class="comparison-pair">
    {#each ["before", "after"] as const as position}
      {@const entry = position === "before" ? before : after}
      {@const other = position === "before" ? after : before}
      <label class="comparison-scan"
        >{position === "before" ? "Before scan" : "After scan"}
        <select
          aria-label={position === "before" ? "Before scan" : "After scan"}
          value={entry?.id ?? ""}
          disabled={busy || !choices.length}
          onchange={(event) => {
            if (!busy)
              selection = {
                ...selected,
                [`${position}Id`]: event.currentTarget.value,
              };
          }}
        >
          <option value="">Choose a scan</option>
          {#each optionsFor(other) as item}<option value={item.id}
              >{scanLabel(item)}</option
            >{/each}
        </select>
        {#if entry}
          {@const time = timestamps(entry)}
          <small class="comparison-scan-path" title={entry.rootPath}
            >{entry.rootPath}</small
          >
          <time
            class="comparison-scan-time"
            datetime={time.scanned.iso}
            title={`Scan completed ${time.scanned.absolute}`}
            >Scanned {time.scanned.label}</time
          >
          {#if time.duplicate}<small
              class="comparison-scan-time"
              title={`Saved ${time.stored.absolute}`}
              >Saved {time.stored.label} · saved result {time.copy + 1}</small
            >{/if}
        {/if}
      </label>
    {/each}
    <div class="comparison-pair-actions">
      <button
        disabled={busy || !before || !after || before.id === after.id}
        onclick={() => {
          if (!busy && before && after && before.id !== after.id)
            selection = { beforeId: after.id, afterId: before.id };
        }}>Swap scans</button
      >
      <button
        class="primary"
        disabled={busy || !compatible}
        onclick={() => {
          if (!compatible || busy || !before || !after) return;
          selection = { beforeId: before.id, afterId: after.id };
          onCompare(before.id, after.id);
        }}>Compare scans</button
      >
    </div>
  </div>
  {#if !ordered.length}<p class="comparison-empty">
      Save two scans of the same location to compare them.
    </p>
  {:else if !before || !after}<p class="comparison-empty">
      Choose two saved scans of the same location.
    </p>
  {:else if before.id === after.id}<p class="comparison-empty">
      Choose two different scans.
    </p>
  {:else if before.rootPath !== after.rootPath}<p class="comparison-empty">
      Both scans must cover the same location.
    </p>{/if}
  {#if before && after && before.id !== after.id && before.completedAtMs >= after.completedAtMs}
    <p class="comparison-order-note">
      {before.completedAtMs === after.completedAtMs
        ? "These scans have the same recorded completion time."
        : "The Before scan finished later than the After scan."} Positive changes
      always mean After − Before, using the selected order.
    </p>
  {/if}
  {#if progress}<p role="status" class="library-progress">
      {phaseLabels[progress.phase] ?? "Comparing saved scans"}… · {bytes(
        progress.bytes,
      )} processed
    </p>{/if}
  {#if result && !currentResult}<p class="comparison-empty">
      You picked new scans. Click Compare scans to update.
    </p>{/if}
  {#if currentResult}
    <p class="comparison-root">{currentResult.rootPath}</p>
    <div class="comparison-overview">
      <div class="comparison-net" data-trend={trend(net)}>
        <span
          >Net change · {metric === "allocated" ? "On disk" : "Logical"}</span
        >
        <strong title={signed(net, true)}>{signed(net)}</strong>
        <b
          >{net > 0
            ? "Increase"
            : net < 0
              ? "Decrease"
              : "No net size change"}</b
        >
        <small>After − Before · entire location</small>
      </div>
      <dl class="comparison-sizes">
        <div>
          <dt>Before · {metric === "allocated" ? "On disk" : "Logical"}</dt>
          <dd title={bytes(metricSize(currentResult.before), 1)}>
            {compactBytes(metricSize(currentResult.before), 1)}
          </dd>
        </div>
        <div>
          <dt>After · {metric === "allocated" ? "On disk" : "Logical"}</dt>
          <dd title={bytes(metricSize(currentResult.after), 1)}>
            {compactBytes(metricSize(currentResult.after), 1)}
          </dd>
        </div>
      </dl>
    </div>
    <div class="comparison-coverage" aria-label="Scan coverage">
      {#each ["before", "after"] as const as position}
        {@const totals = currentResult[position]}
        <p
          data-coverage={incomplete(totals)
            ? "incomplete"
            : totals.excluded > 0
              ? "excluded"
              : "complete"}
        >
          <strong
            >{position === "before" ? "Before" : "After"}: {coverage(
              totals,
            )}</strong
          >{totals.errors
            ? ` · ${totals.errors.toLocaleString()} recorded issue ${totals.errors === 1 ? "event" : "events"}`
            : ""}{totals.excluded
            ? ` · ${totals.excluded.toLocaleString()} excluded ${totals.excluded === 1 ? "location" : "locations"}`
            : ""}
        </p>
      {/each}
    </div>
    {#if currentResult.warnings.length}
      <details class="comparison-warnings" aria-label="Comparison warnings">
        <summary>Comparison notes ({currentResult.warnings.length})</summary>
        <ul>
          {#each currentResult.warnings as warning}<li>{warning}</li>{/each}
        </ul>
      </details>
      <p class="comparison-notes-summary">
        Notes may warn about missed folders, how sizes are counted, or whether it's the same place.
      </p>
    {/if}
    <div class="comparison-controls">
      {#if logicalSize.shown}
        <label
          >Size metric<select
            aria-label="Comparison size metric"
            value={metric}
            onchange={(event) =>
              (chosenMetric = event.currentTarget.value as typeof metric)}
            ><option value="allocated">On disk</option><option value="logical"
              >Logical size</option
            ></select
          ></label
        >
      {/if}
      <label
        >Entry type<select
          aria-label="Comparison entry type"
          value={kind}
          onchange={(event) =>
            (kind = event.currentTarget.value as typeof kind)}
          ><option value="files">Files &amp; other entries</option><option
            value="folders">Folders</option
          ></select
        ></label
      >
    </div>
    <div class="comparison-views" role="group" aria-label="Change views">
      {#each views as view}
        {@const count =
          currentResult.rankings.find(
            (item) =>
              item.metric === metric &&
              item.kind === kind &&
              item.direction === view.direction,
          )?.total ?? 0}
        <button
          data-direction={view.direction}
          aria-pressed={direction === view.direction}
          onclick={() => (direction = view.direction)}
          >{view.label} <strong>{count.toLocaleString()}</strong></button
        >
      {/each}
    </div>
    <p class="comparison-view-copy">
      {activeView.copy} These views overlap; do not add their counts together.
    </p>
    <h3 class="comparison-ranking-title">{activeView.heading}</h3>
    <p class="comparison-limit">
      Showing {rows.length.toLocaleString()} of {total.toLocaleString()} matching
      {kind === "files" ? "files & other entries" : "folders"}. {direction ===
        "growth" || direction === "shrinkage"
        ? "Up to 100 largest changes are shown."
        : "Up to 100 entries are shown."}
    </p>
    {#if rows.length}<div class="comparison-table-wrap">
        <table aria-label="Ranked changes">
          <thead
            ><tr
              ><th>Entry</th><th>Status</th><th>Before</th><th>After</th><th
                >Change</th
              ></tr
            ></thead
          >
          <tbody
            >{#each rows as row, index}
              {@const parts = pathParts(row.path)}
              {@const beforeSize =
                metric === "allocated"
                  ? row.beforeAllocatedBytes
                  : row.beforeLogicalBytes}
              {@const afterSize =
                metric === "allocated"
                  ? row.afterAllocatedBytes
                  : row.afterLogicalBytes}
              {@const delta =
                metric === "allocated" ? row.allocatedDelta : row.logicalDelta}
              <tr
                ><td class="comparison-path" title={row.path}
                  ><strong class="comparison-name">{parts.name}</strong><span
                    class="comparison-parent">{parts.parent}</span
                  ><small class="comparison-row-kind"
                    >{row.kind === "directory"
                      ? "Folder"
                      : row.kind === "symlink"
                        ? "Symbolic link"
                        : row.kind === "file"
                          ? "File"
                          : "Other entry"}</small
                  ></td
                >
                <td
                  >{row.status === "added"
                    ? "Added"
                    : row.status === "removed"
                      ? "Removed"
                      : "Changed"}</td
                >
                <td
                  title={row.status === "added"
                    ? "Not present in Before scan"
                    : bytes(beforeSize, 1)}
                  >{row.status === "added"
                    ? "—"
                    : compactBytes(beforeSize, 1)}</td
                >
                <td
                  title={row.status === "removed"
                    ? "Not present in After scan"
                    : bytes(afterSize, 1)}
                  >{row.status === "removed"
                    ? "—"
                    : compactBytes(afterSize, 1)}</td
                >
                <td
                  class="comparison-delta"
                  data-trend={trend(delta)}
                  title={signed(delta, true)}
                  ><strong>{signed(delta)}</strong><small
                    >{delta > 0
                      ? "Grew"
                      : delta < 0
                        ? "Shrank"
                        : "No size change"}</small
                  ></td
                >
              </tr>
            {/each}</tbody
          >
        </table>
      </div>
    {:else}<p class="comparison-empty">
        {direction === "growth" || direction === "shrinkage"
          ? `No ${kind === "files" ? "files & other entries" : "folders"} with ${direction} in ${metric === "allocated" ? "on-disk" : "logical"} size.`
          : direction === "changed"
            ? `No ${kind === "files" ? "files & other entries" : "folders"} with changed metadata.`
            : `No ${direction} ${kind === "files" ? "files & other entries" : "folders"}.`}
      </p>{/if}
    <div class="comparison-counts" aria-label="Entry changes">
      <span>Added <strong>{currentResult.added.toLocaleString()}</strong></span
      ><span
        >Removed <strong>{currentResult.removed.toLocaleString()}</strong></span
      ><span
        >Changed <strong>{currentResult.changed.toLocaleString()}</strong></span
      ><span
        >Unchanged <strong>{currentResult.unchanged.toLocaleString()}</strong
        ></span
      >
    </div>
    <p class="comparison-explanation">
      These counts include every kind of entry but not the top folder. The location totals do include it. Changed means the saved details differ. File contents and edit times are not compared. Folder changes count everything inside them, so don't add them up.
    </p>
    <details class="comparison-total-details">
      <summary>Full scan totals</summary>
      <div class="comparison-totals">
        <table aria-label="Scan totals">
          <thead
            ><tr
              ><th>Measure</th><th>Before</th><th>After</th><th>Change</th></tr
            ></thead
          >
          <tbody>
            {#each sizeMetrics as sizeMetric}
              {@const a =
                sizeMetric === "allocated"
                  ? currentResult.before.allocatedBytes
                  : currentResult.before.logicalBytes}
              {@const b =
                sizeMetric === "allocated"
                  ? currentResult.after.allocatedBytes
                  : currentResult.after.logicalBytes}
              {@const delta =
                sizeMetric === "allocated"
                  ? currentResult.allocatedDelta
                  : currentResult.logicalDelta}
              <tr
                ><th
                  >{sizeMetric === "allocated" ? "On disk" : "Logical size"}</th
                ><td title={bytes(a, 1)}>{compactBytes(a, 1)}</td><td
                  title={bytes(b, 1)}>{compactBytes(b, 1)}</td
                ><td title={signed(delta, true)}>{signed(delta)}</td></tr
              >
            {/each}
            <tr
              ><th>Entries</th><td
                >{currentResult.before.entries.toLocaleString()}</td
              ><td>{currentResult.after.entries.toLocaleString()}</td><td
                >{(
                  currentResult.after.entries - currentResult.before.entries
                ).toLocaleString(undefined, { signDisplay: "exceptZero" })}</td
              ></tr
            >
            <tr
              ><th>Coverage</th><td>{coverage(currentResult.before)}</td><td
                >{coverage(currentResult.after)}</td
              ><td>—</td></tr
            >
          </tbody>
        </table>
      </div>
    </details>
  {/if}
</section>
