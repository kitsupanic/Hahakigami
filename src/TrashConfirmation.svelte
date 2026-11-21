<script lang="ts">
  import Icon from "./Icon.svelte";
  import { dialogFocus } from "./dialogFocus";
  import { bytes, type NodeView } from "./types";
  import { compactBytes, countLabel } from "./uiFormat";

  let {
    items,
    queued = false,
    selectedCount,
    busy,
    opener,
    onCancel,
    onConfirm,
  }: {
    items: NodeView[];
    queued?: boolean;
    selectedCount?: number;
    busy: boolean;
    opener: HTMLElement | null;
    onCancel: () => void;
    onConfirm: () => void;
  } = $props();
  let folders = $derived(
    items.filter((item) => item.kind === "directory").length,
  );
  let files = $derived(items.filter((item) => item.kind === "file").length);
  let links = $derived(items.filter((item) => item.kind === "symlink").length);
  let others = $derived(items.length - folders - files - links);
  let kindCounts = $derived(
    [
      folders ? countLabel(folders, "folder") : "",
      files ? countLabel(files, "file") : "",
      links ? countLabel(links, "symbolic link") : "",
      others ? countLabel(others, "other item") : "",
    ]
      .filter(Boolean)
      .join(" · ") || "No items",
  );
  let allocated = $derived(total());
  let partial = $derived(items.some((item) => item.incomplete));
  function validSize(value: number) {
    return Number.isSafeInteger(value) && value >= 0;
  }
  function total(): number | null {
    let result = 0;
    for (const item of items) {
      if (!validSize(item.allocatedBytes) || !validSize(result + item.allocatedBytes)) return null;
      result += item.allocatedBytes;
    }
    return result;
  }
  function sizeLabel(value: number | null) {
    return value !== null && validSize(value)
      ? compactBytes(value)
      : "Unavailable";
  }
  function sizeTitle(value: number | null) {
    return value !== null && validSize(value)
      ? bytes(value)
      : "Recorded size is unavailable";
  }
  function kindLabel(item: NodeView) {
    return item.kind === "directory"
      ? "Folder and contents"
      : item.kind === "symlink"
        ? "Symbolic link only"
        : item.kind === "file"
          ? "File"
          : "Other item";
  }
</script>

<div class="modal-backdrop">
  <div
    class="modal trash-modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="trash-title"
    aria-busy={busy}
    tabindex="-1"
    use:dialogFocus={{
      opener,
      initialFocus: "[data-trash-cancel]:not(:disabled)",
      onClose: () => {
        if (!busy) onCancel();
      },
    }}
  >
    <h2 id="trash-title">
      Move {countLabel(items.length, "item")} to Trash?
    </h2>
    <p class="trash-intro">
      {queued ? "Ready items from the current scan" : "Selected items"}
    </p>
    {#if queued}
      <p class="trash-scope-note">
        Only these ready items will move. Other items stay in the cleanup queue.
      </p>
    {:else if selectedCount !== undefined && Number.isSafeInteger(selectedCount) && selectedCount > items.length}
      <p class="trash-scope-note">
        {selectedCount.toLocaleString()} selected; {countLabel(
          items.length,
          "item",
        )} to move. Selected descendants are included through their parent folders.
      </p>
    {/if}
    <dl class="trash-summary">
      <div>
        <dt>Items to move</dt>
        <dd>{items.length.toLocaleString()}</dd>
      </div>
      <div>
        <dt>On disk</dt>
        <dd title={sizeTitle(allocated)}>{sizeLabel(allocated)}</dd>
      </div>
    </dl>
    <p class="trash-kind-counts">{kindCounts}</p>
    {#if folders > 0}
      <p class="trash-folder-warning">
        Moving a folder moves everything inside it, even things that weren't scanned.
      </p>
    {/if}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (The bounded review list must be keyboard scrollable.) -->
    <ul class="trash-items" aria-label="Items to move to Trash" tabindex="0">
      {#each items as item (item.id)}
        <li class="trash-item">
          <Icon
            name={item.kind === "directory" ? "folder" : "file"}
            size={20}
          />
          <div class="trash-item-copy">
            <strong>{item.name}</strong>
            <small class="trash-item-path">{item.path}</small>
            <small class="trash-item-kind"
              >{kindLabel(item)}{item.incomplete
                ? " · Partial recorded size"
                : ""}</small
            >
          </div>
          <span class="trash-item-size" title={sizeTitle(item.allocatedBytes)}
            >{sizeLabel(item.allocatedBytes)}</span
          >
        </li>
      {/each}
    </ul>
    <p class="trash-recovery-note">
      You can restore these items from Trash in Finder.
    </p>
    <p class="trash-size-note">
      {#if partial}Some recorded sizes are partial. Folder sizes include only
        recorded contents.
      {/if}Space is freed only when you empty the Trash, and can be less than
      these sizes.
    </p>
    <div class="trash-actions">
      <button
        data-trash-cancel
        disabled={busy}
        onclick={() => {
          if (!busy) onCancel();
        }}>Cancel</button
      >
      <button
        class="danger"
        disabled={busy || !items.length}
        onclick={() => {
          if (!busy && items.length) onConfirm();
        }}>{busy ? "Moving…" : "Move to Trash"}</button
      >
    </div>
  </div>
</div>
