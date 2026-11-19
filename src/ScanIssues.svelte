<script lang="ts">
  import { coverageLabel } from "./scanCoverage";
  import { invoke } from "@tauri-apps/api/core";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import type { ScanIssuePage, ScanIssueKind, ScanSummary } from "./types";
  const limit = 100;
  import { untrack } from "svelte";
  import { latestOnly } from "./latestQuery";
  // The Coverage tab of ScanDetails. Details load only while the tab is shown.
  let { summary, active = false }: { summary: ScanSummary; active?: boolean } =
    $props();
  let scanIdentity = $derived(summary.scanId);
  let restricted = $derived(
    Math.min(summary.errors, Math.max(0, summary.restrictedSystemIssues ?? 0)),
  );
  let other = $derived(Math.max(0, summary.errors - restricted));
  let initialKind: ScanIssueKind = $derived(
    !other && restricted ? "restricted" : "other",
  );
  let incomplete = $derived(!!coverageLabel(summary));
  // There is no kind picker: follow the category that has entries, so real errors that
  // arrive after restricted ones still show.
  let kind: ScanIssueKind = $derived(initialKind);
  let offset = $state.raw(0);
  let page: ScanIssuePage | null = $state.raw(null);
  let loading = $state.raw(false);
  let error = $state.raw("");
  let retry = $state.raw(0);
  let generation = 0;
  // A new category starts on its first page. Runs before the load effect reads it.
  $effect.pre(() => {
    kind;
    untrack(() => (offset = 0));
  });
  let copying = $state.raw(false);
  let copyMessage = $state.raw("");
  let copyError = $state.raw("");
  let copyGeneration = 0;
  $effect(() => {
    scanIdentity;
    return untrack(() => {
      ++copyGeneration;
      copying = false;
      copyMessage = "";
      copyError = "";
      return () => {
        ++copyGeneration;
      };
    });
  });
  async function copyErrors() {
    const snapshot = summary;
    const current = ++copyGeneration;
    const stale = () => current !== copyGeneration;
    copying = true;
    copyMessage = "";
    copyError = "";
    try {
      const issues: ScanIssuePage["items"] = [];
      let first: ScanIssuePage | null = null;
      do {
        const result = await invoke<ScanIssuePage>("get_scan_issues", {
          scanId: snapshot.scanId,
          kind: "errors",
          offset: issues.length,
          limit,
        });
        if (stale()) return;
        if (
          result.scanId !== snapshot.scanId ||
          result.nodeId !== snapshot.rootId ||
          result.kind !== "errors" ||
          result.offset !== issues.length ||
          !Number.isSafeInteger(result.total) ||
          result.total < 0 ||
          !Number.isSafeInteger(result.retainedTotal) ||
          result.retainedTotal < 0 ||
          result.retainedTotal > 10_000 ||
          result.retainedTotal > result.total ||
          result.items.length > limit
        ) {
          throw new Error(
            "Error response does not match the requested scan page.",
          );
        }
        first ??= result;
        if (
          result.retainedTotal < first.retainedTotal ||
          result.total < first.total
        ) {
          throw new Error(
            "Scan error details changed while copying. Try again.",
          );
        }
        const remaining = first.retainedTotal - issues.length;
        if (remaining > 0 && result.items.length < Math.min(limit, remaining)) {
          throw new Error("Scan error details are incomplete. Try again.");
        }
        issues.push(...result.items.slice(0, remaining));
      } while (issues.length < first.retainedTotal);
      if (stale()) return;
      await writeText(
        JSON.stringify(
          {
            app: "Hahakigami",
            version: 1,
            scanId: snapshot.scanId,
            rootNodeId: first.nodeId,
            scanState: snapshot.state,
            totalIssues: first.total,
            copiedIssues: issues.length,
            omittedIssues: first.total - issues.length,
            truncated: issues.length < first.total,
            note: "Counts are issue events, not unreadable files. This report contains the retained errors when copying started; a running or cancelled scan is partial. Expected exclusions are not included.",
            issues,
          },
          null,
          2,
        ),
      );
      if (!stale())
        copyMessage = `Copied ${issues.length.toLocaleString()} scan ${issues.length === 1 ? "issue" : "issues"}${issues.length < first.total ? ` of ${first.total.toLocaleString()} recorded events (details truncated)` : ""}.`;
    } catch (e) {
      if (!stale()) copyError = `Could not copy errors: ${String(e)}`;
    } finally {
      if (!stale()) copying = false;
    }
  }
  let count = $derived(
    kind === "other"
      ? other
      : kind === "restricted"
        ? restricted
        : kind === "errors"
          ? summary.errors
          : (summary.excluded ?? 0),
  );
  const queryIssues = latestOnly();
  $effect(() => {
    scanIdentity;
    kind;
    offset;
    active;
    count;
    retry;
    return untrack(() => {
      const scanId = scanIdentity,
        requestedKind = kind,
        requestedOffset = offset;
      const current = ++generation;
      error = "";
      // A count change refetches the shown page in place; any other request drops it.
      if (
        !active ||
        page?.scanId !== scanId ||
        page.kind !== requestedKind ||
        page.offset !== requestedOffset
      )
        page = null;
      if (!active) return;
      loading = !page;
      queryIssues(() => {
        if (current !== generation) return Promise.resolve();
        return invoke<ScanIssuePage>("get_scan_issues", {
          scanId,
          kind: requestedKind,
          offset: requestedOffset,
          limit,
        })
        .then((result) => {
          if (current !== generation) return;
          if (
            result.scanId !== scanId ||
            result.kind !== requestedKind ||
            result.offset !== requestedOffset
          )
            throw new Error(
              "Issue response does not match the requested scan page.",
            );
          page = result;
        })
        .catch((e) => {
          if (current === generation) error = String(e);
        })
        .finally(() => {
          if (current === generation) loading = false;
        });
      });
      return () => {
        ++generation;
      };
    });
  });
</script>

<section
  class="scan-issues"
  class:has-other-issues={other > 0}
  class:partial-coverage={incomplete}
  aria-label="Coverage details"
>
    {#if copyMessage}<p role="status">{copyMessage}</p>{/if}{#if copyError}<p
        role="alert"
      >
        {copyError}
      </p>{/if}{#if active}<div class="scan-status-details">
        <strong
          >{coverageLabel(summary)}</strong
        >
        {#if incomplete && !summary.errors}<p class="scan-issue-warning">
            Some places were not scanned. No details were saved about why.
          </p>{/if}
        {#if summary.errors > 0}<div class="scan-issue-actions detail-actions">
            <button disabled={copying} onclick={() => void copyErrors()}
              >{copying ? "Copying errors…" : "Copy errors"}</button
            >
          </div>{/if}
        {#if other > 0}<p class="scan-issue-warning">
            Some data is missing because the scan hit problems. See each reason below. Full Disk Access can't get past file permissions or every macOS protection. An access-denied error doesn't tell you which one blocked the scan.
          </p>{/if}
        <p class="scan-issue-note">
          These counts are problems or skipped places, not unreadable files. One folder problem can affect everything inside it.
        </p>
        {#if loading && !page}<p role="status">
            Loading scan details…
          </p>{/if}{#if error}<div role="alert">
            {error}{" "}<button
              onclick={() => (retry = ((value) => value + 1)(retry))}
              >Retry scan details</button
            >
          </div>{/if}{#if page}{#if page.retainedTotal < page.total}<p
              class="scan-issue-note"
            >
              Showing retained details for{" "}{page.retainedTotal.toLocaleString()}
              of{" "}{page.total.toLocaleString()}{" "}events. Additional
              details were not retained.
            </p>{/if}{#if !page.items.length}<p>
              No details were kept for this page.
            </p>{/if}
          <ol class="scan-issue-list" start={offset + 1}>
            {#each page.items as issue, index}<li>
                <code>{issue.path}</code>
                <p>{issue.message}</p>
                {#if issue.errorKind || issue.rawOsError != null}<small
                    >{issue.errorKind}{issue.rawOsError != null
                      ? `${issue.errorKind ? " · " : ""}OS error ${issue.rawOsError}`
                      : ""}</small
                  >{/if}
              </li>{/each}
          </ol>
          <div class="scan-issue-pagination">
            <span
              >{page.items.length
                ? `${offset + 1}–${offset + page.items.length} of ${page.retainedTotal.toLocaleString()} retained details`
                : "0 details"}</span
            ><button
              disabled={loading || offset === 0}
              onclick={() =>
                (offset = ((value) => Math.max(0, value - limit))(offset))}
              >Previous details</button
            ><button
              disabled={loading || offset + limit >= page.retainedTotal}
              onclick={() => (offset = ((value) => value + limit)(offset))}
              >Next details</button
            >
          </div>{/if}
      </div>{/if}
</section>

<style>
  .scan-issues {
    position: relative;
    flex: 0 0 auto;
    margin: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    font-size: var(--font-caption);
    min-width: 0;
  }
  .scan-issues .scan-issue-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    margin: 0;
    gap: 6px;
  }
  .scan-status-details > strong {
    display: block;
    font-size: var(--font-title3);
    color: var(--text);
  }
  .scan-issues .detail-actions {
    margin-top: 8px;
  }
  .scan-issue-actions button {
    padding: 4px 8px;
    font-size: var(--font-caption);
  }
</style>
