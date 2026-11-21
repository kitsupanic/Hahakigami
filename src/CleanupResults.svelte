<script lang="ts">
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
  import type { CleanupReport } from "./cleanupTypes";
  import { compactBytes, countLabel } from "./uiFormat";

  let {
    report,
    queueOpen = false,
    disabled = false,
    opener = null,
    onQueue,
    onRetry,
    onDismiss,
  }: {
    report: CleanupReport;
    queueOpen?: boolean;
    disabled?: boolean;
    opener?: HTMLElement | null;
    onQueue: () => void;
    onRetry: () => void;
    onDismiss: () => void;
  } = $props();
  const itemCount = (count: number) => countLabel(count, "item");
  const title = $derived(
    report.state === "unconfirmed"
      ? "Cleanup result unconfirmed"
      : !report.movedCount
        ? "No items moved to Trash"
        : report.failedCount
          ? `Moved ${report.movedCount.toLocaleString()} of ${itemCount(report.movedCount + report.failedCount)} to Trash`
          : `Moved ${itemCount(report.movedCount)} to Trash`,
  );
  // Sizes come from the reviewed scan; they are only known with item-by-item results.
  const movedBytes = $derived(
    report.movedItems.reduce((sum, item) => sum + item.allocatedBytes, 0),
  );
  const needsAttention = $derived(
    report.state === "unconfirmed" || (report.failedCount ?? 0) > 0,
  );
  // A clean run says everything in the headline; the count table is for results that need checking.
  const showSummary = $derived(
    needsAttention || report.queueState === "unconfirmed",
  );
  const queueLine = $derived(
    report.queueState === "checking"
      ? "Checking the queue…"
      : report.queueState === "confirmed" && report.remainingQueued
        ? `${itemCount(report.remainingQueued)} still queued across all locations`
        : null,
  );
</script>

<Modal
  class="cleanup-modal"
  labelledby="cleanup-result-title"
  initialFocus="[data-cleanup-done]"
  {opener}
  onClose={onDismiss}
>
  <section
    class="cleanup-results"
    class:needs-attention={needsAttention}
    data-state={report.state}
  >
    <div class="cleanup-result-heading">
      <span class="cleanup-result-icon">
        <Icon name={needsAttention ? "info" : "check"} size={18} />
      </span>
      <div>
        <h2 id="cleanup-result-title">
          <span role="status" aria-live="polite" aria-atomic="true">{title}</span>
        </h2>
        {#if report.movedCount && report.movedItems.length}
          <p class="cleanup-result-size">
            <strong>{compactBytes(movedBytes)}</strong>
            <span>on disk</span>
          </p>
        {/if}
        {#if report.movedCount}
          <p class="cleanup-result-hint">
            Empty the Trash in Finder to free the space.
          </p>
        {/if}
        {#if report.movedCount && report.movedItems.length}
          <p class="cleanup-result-caveat">
            You may get back less space. Cloned files, hard links and compression can cause this.
          </p>
        {/if}
        <p class="cleanup-result-source">
          {[
            report.source === "queue"
              ? "From the cleanup queue"
              : "From your reviewed selection",
            !showSummary && queueLine,
          ]
            .filter(Boolean)
            .join(" · ")}
        </p>
      </div>
    </div>
    {#if showSummary}<dl class="cleanup-result-summary">
      <div>
        <dt>Moved to Trash</dt>
        <dd>{report.movedCount?.toLocaleString() ?? "Unconfirmed"}</dd>
      </div>
      <div>
        <dt>Failed to move</dt>
        <dd>{report.failedCount?.toLocaleString() ?? "Unconfirmed"}</dd>
      </div>
      <div>
        <dt>Still queued, all locations</dt>
        <dd>
          {report.queueState === "checking"
            ? "Checking…"
            : report.queueState === "confirmed"
              ? report.remainingQueued?.toLocaleString()
              : "Unconfirmed"}
        </dd>
      </div>
    </dl>{/if}
    {#if report.state === "unconfirmed"}
      <p class="cleanup-result-warning">
        Couldn't tell if the cleanup worked. Check Trash in Finder and the queue before you try again.
      </p>
    {/if}
    {#if report.message}<p class="cleanup-result-message">
        {report.message}
      </p>{/if}
    {#if report.persistenceWarning}
      <p class="cleanup-result-warning">
        The queue may be out of date. Moved items may still show in it.
      </p>
      <p class="cleanup-result-message">{report.persistenceWarning}</p>
    {/if}
    {#if report.queueState === "unconfirmed"}
      <p class="cleanup-result-warning">
        Couldn't check how many items are left in the queue.
      </p>
      {#if report.queueError}<p class="cleanup-result-message">
          {report.queueError}
        </p>{/if}
    {/if}
    {#if report.queueWarning && report.queueWarning !== report.persistenceWarning}
      <p class="cleanup-result-message">{report.queueWarning}</p>
    {/if}
    <div class="cleanup-result-footer">
      {#key report.id}
        <details class="cleanup-result-details">
          <summary>View moved and failed items</summary>
          {#if report.detailsAvailable}
            {#if report.failedItems.length}
              <h3>Could not move to Trash</h3>
              <!-- svelte-ignore a11y_no_noninteractive_tabindex (the full result list supports keyboard scrolling) -->
              <ul
                class="cleanup-result-items"
                aria-label="Items that failed to move to Trash"
                tabindex="0"
              >
                {#each report.failedItems as failed, index (index)}
                  <li>
                    <Icon
                      name={failed.item.kind === "directory" ? "folder" : "file"}
                      size={16}
                    />
                    <div>
                      <strong>{failed.item.name}</strong><span
                        class="cleanup-result-path">{failed.item.path}</span
                      >
                      <p>{failed.message}</p>
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if report.movedItems.length}
              <h3>Moved to Trash</h3>
              <!-- svelte-ignore a11y_no_noninteractive_tabindex (the full result list supports keyboard scrolling) -->
              <ul
                class="cleanup-result-items"
                aria-label="Items moved to Trash"
                tabindex="0"
              >
                {#each report.movedItems as item, index (index)}
                  <li>
                    <Icon
                      name={item.kind === "directory" ? "folder" : "file"}
                      size={16}
                    />
                    <div>
                      <strong>{item.name}</strong><span
                        class="cleanup-result-path">{item.path}</span
                      >{#if item.kind === "directory"}<small
                          >Folder and contents</small
                        >{/if}
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if report.state === "confirmed"}
            <p class="cleanup-result-message">
              Can't show each item for this cleanup. The totals above are confirmed.
            </p>
            {#each report.failures as message, index (index)}<p
                class="cleanup-result-message"
              >
                {message}
              </p>{/each}
          {:else}
            <p class="cleanup-result-message">
              No single item could be confirmed.
            </p>
          {/if}
          <p class="cleanup-result-note">
            You can get moved items back from Trash in Finder.
          </p>
        </details>
      {/key}
      <div class="cleanup-result-actions">
        {#if report.queueState === "unconfirmed"}<button
            {disabled}
            onclick={onRetry}>Retry queue status</button
          >{/if}
        {#if !queueOpen}<button {disabled} onclick={onQueue}>Review queue</button
          >{/if}
        <button class="primary" data-cleanup-done onclick={onDismiss}>Done</button>
      </div>
    </div>
  </section>
</Modal>
