<script lang="ts">
  import { onMount, tick } from "svelte";
  import { compactBytes, countLabel } from "./uiFormat";
  import type { FileTypeScope } from "./createFileTypes.svelte";
  import {
    fileTypeColor,
    fileTypeDescription,
    fileTypeLabel,
    type FileTypeStats,
  } from "./fileTypes";
  let {
    stats,
    scope,
    selectedExtension,
    loading = false,
    error = "",
    highlightPending = false,
    highlightError = "",
    disabled = false,
    scopePath = "",
    running = false,
    onScopeChange,
    onSelect,
    onShowFiles,
    onRetry,
  }: {
    stats: FileTypeStats | null;
    scope: FileTypeScope;
    selectedExtension: string | null;
    loading?: boolean;
    error?: string;
    highlightPending?: boolean;
    highlightError?: string;
    disabled?: boolean;
    scopePath?: string;
    running?: boolean;
    onScopeChange: (scope: FileTypeScope) => void;
    onSelect: (extension: string | null) => void;
    onShowFiles: () => void;
    onRetry: () => void;
  } = $props();
  const ROW_HEIGHT = 64,
    OVERSCAN = 5;
  let search = $state("");
  let scroll = $state(0);
  let viewHeight = $state(320);
  let list: HTMLDivElement | undefined;
  let focusRequest = 0;
  onMount(() => {
    const element = list!;
    let visible = false;
    const measure = () => {
      if (element.clientHeight <= 0) {
        visible = false;
        return;
      }
      viewHeight = element.clientHeight;
      if (!visible) element.scrollTop = scroll;
      visible = true;
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  });
  const total = $derived(stats?.allocatedBytes ?? 0);
  const sortedRows = $derived.by(() =>
    (stats?.items ?? [])
      .slice()
      .sort(
        (a, b) =>
          b.allocatedBytes - a.allocatedBytes ||
          b.files - a.files ||
          a.extension.localeCompare(b.extension),
      ),
  );
  const searchTerms = $derived(
    new Map(
      sortedRows.map((item) => [
        item.extension,
        `${fileTypeLabel(item.extension)} ${fileTypeDescription(item.extension)}`.toLowerCase(),
      ]),
    ),
  );
  const rows = $derived.by(() => {
    const query = search.trim().toLowerCase();
    return query
      ? sortedRows.filter((item) =>
          searchTerms.get(item.extension)!.includes(query),
        )
      : sortedRows;
  });
  const listContext = $derived(
    `${scope}:${scopePath}:${search.trim().toLowerCase()}`,
  );
  $effect(() => {
    listContext;
    focusRequest++;
    scroll = 0;
    if (list) list.scrollTop = 0;
  });
  const start = $derived(
    Math.max(
      0,
      Math.min(rows.length - 1, Math.floor(scroll / ROW_HEIGHT) - OVERSCAN),
    ),
  );
  const end = $derived(
    Math.min(
      rows.length,
      start + Math.ceil(viewHeight / ROW_HEIGHT) + OVERSCAN * 2,
    ),
  );
  async function browse(event: KeyboardEvent, index: number) {
    if (event.altKey || event.metaKey || !rows.length || !list) return;
    const page = Math.max(1, Math.floor(viewHeight / ROW_HEIGHT));
    const next =
      event.key === "ArrowDown"
        ? index + 1
        : event.key === "ArrowUp"
          ? index - 1
          : event.key === "PageDown"
            ? index + page
            : event.key === "PageUp"
              ? index - page
              : event.key === "Home"
                ? 0
                : event.key === "End"
                  ? rows.length - 1
                  : null;
    if (next === null) return;
    event.preventDefault();
    const element = list;
    const target = Math.max(0, Math.min(rows.length - 1, next));
    const top = target * ROW_HEIGHT;
    if (top < scroll) scroll = top;
    else if (top + ROW_HEIGHT > scroll + viewHeight)
      scroll = top + ROW_HEIGHT - viewHeight;
    element.scrollTop = scroll;
    const context = listContext,
      request = ++focusRequest;
    const previous = document.activeElement;
    await tick();
    if (
      list !== element ||
      !element.isConnected ||
      request !== focusRequest ||
      context !== listContext ||
      element.closest("[hidden]") ||
      (document.activeElement !== previous &&
        document.activeElement !== document.body)
    )
      return;
    element
      .querySelector<HTMLButtonElement>(`[data-type-index="${target}"]`)
      ?.focus({ preventScroll: true });
  }
  const selected = $derived(
    stats?.items.find((item) => item.extension === selectedExtension),
  );
  const share = (value: number) =>
    `${(total > 0 ? (value / total) * 100 : 0).toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;
</script>

<section class="file-types" aria-label="File type statistics">
  <div class="panel-title">
    <strong>
      File types{" "}<small
        >{stats
          ? `${stats.items.length.toLocaleString()} ${stats.items.length === 1 ? "type" : "types"}`
          : ""}</small
      >
    </strong>
    <!-- One fixed order, so a caption rather than a select. -->
    <small class="file-type-order">Largest first</small>
  </div>
  <div class="file-types-body">
  <div
    class="segmented file-type-scope"
    role="group"
    aria-label="File type scope"
  >
    <button
      class:chosen={scope === "scan"}
      aria-pressed={scope === "scan"}
      {disabled}
      onclick={() => onScopeChange("scan")}>Entire scan</button
    >
    <button
      class:chosen={scope === "folder"}
      aria-pressed={scope === "folder"}
      {disabled}
      onclick={() => onScopeChange("folder")}>Current folder</button
    >
  </div>
  <p class="file-type-total" title={scopePath}>
    <strong>{stats ? compactBytes(total, 1) : "—"}</strong> · On disk
    <span
      >{stats
        ? `${stats.files.toLocaleString()} ${stats.files === 1 ? "file" : "files"}`
        : "Scanned file metadata"}</span
    >
  </p>
  <input
    class="file-type-search"
    aria-label="Search file types"
    placeholder="Search extensions…"
    type="search"
    bind:value={search}
  />
  <div class="file-type-columns" aria-hidden="true">
    <span>Type</span><span>On disk</span>
  </div>
  <div
    class="file-type-list"
    aria-busy={loading}
    bind:this={list}
    onscroll={(event) => {
      if (!event.currentTarget.closest("[hidden]"))
        scroll = event.currentTarget.scrollTop;
    }}
  >
    {#if error}
      <p class="file-type-error" role="alert">
        Could not load file types. {error}
      </p>
      <button onclick={onRetry}>Retry file types</button>
    {:else if loading}
      <p class="file-type-status" role="status">Loading file types…</p>
    {:else if rows.length === 0}
      <p class="file-type-status">
        {search.trim()
          ? "No file types match your search."
          : "No scanned files in this scope."}
      </p>
    {:else}
      <ul
        aria-label="File types by size"
        style:height={`${rows.length * ROW_HEIGHT}px`}
      >
        {#each rows.slice(start, end) as item, i (item.extension)}
          <li
            aria-posinset={start + i + 1}
            aria-setsize={rows.length}
            style:top={`${(start + i) * ROW_HEIGHT}px`}
            style:height={`${ROW_HEIGHT}px`}
          >
            <button
              class="file-type-row"
              class:active={selectedExtension === item.extension}
              aria-pressed={selectedExtension === item.extension}
              data-type-index={start + i}
              aria-label={`${fileTypeLabel(item.extension)}, ${compactBytes(item.allocatedBytes)}, ${countLabel(item.files, "file")}`}
              title={fileTypeDescription(item.extension)}
              {disabled}
              onkeydown={(event) => void browse(event, start + i)}
              onclick={() =>
                onSelect(
                  selectedExtension === item.extension ? null : item.extension,
                )}
            >
              <span
                class="file-type-swatch"
                style:background={fileTypeColor(item.extension)}
                aria-hidden="true"
              ></span>
              <span class="file-type-name"
                ><strong>{fileTypeLabel(item.extension)}</strong>
                <span class="file-type-description"
                  >{fileTypeDescription(item.extension)}</span
                >
                <small
                  >{item.files.toLocaleString()}
                  {item.files === 1 ? "file" : "files"}</small
                ></span
              >
              <span class="file-type-size"
                ><strong>{compactBytes(item.allocatedBytes, 1)}</strong><small
                  >{share(item.allocatedBytes)}</small
                ></span
              >
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
  {#if selectedExtension !== null}
    <div class="file-type-selection">
      <div>
        <span
          class="file-type-swatch"
          style:background={fileTypeColor(selectedExtension)}
          aria-hidden="true"
        ></span>
        <strong>{fileTypeLabel(selectedExtension)} highlighted</strong>
        <button
          class="file-type-clear"
          aria-label="Clear file type highlight"
          onclick={() => onSelect(null)}>Clear</button
        >
      </div>
      <p>
        {selected
          ? `${selected.files.toLocaleString()} ${selected.files === 1 ? "file" : "files"} · ${compactBytes(selected.allocatedBytes, 1)}`
          : loading
            ? "Loading type totals…"
            : "No matching files in this scope"}
      </p>
      {#if highlightPending}<small role="status">Updating map highlights…</small
        >{/if}
      {#if highlightError}<p class="file-type-error" role="alert">
          Could not highlight files. {highlightError}
        </p>
        <button onclick={onRetry}>Retry highlights</button>{/if}
      <button
        class="file-type-show-files"
        disabled={disabled || !selected || loading}
        onclick={onShowFiles}>Show matching files</button
      >
    </div>
  {/if}
  <p class="file-type-note">
    {#if running}Scan in progress ·
    {/if}{#if stats?.incomplete}<span class="file-type-partial"
        >Limited coverage ·
      </span>{/if}Scanned file totals; folder metadata and links excluded.
  </p>
  </div>
</section>

<style>
  .file-types {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .file-types-body {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    padding: 10px 14px 14px;
  }
  .file-type-total,
  .file-type-status,
  .file-type-note {
    color: var(--muted);
    font-size: var(--font-caption);
  }
  .file-type-scope > button {
    flex: 1 1 0;
  }
  .file-type-total {
    margin: 8px 0;
    line-height: 1.4;
  }
  .file-type-total strong {
    color: var(--text);
  }
  .file-type-total span {
    display: block;
    font-size: var(--font-caption);
  }
  .file-type-search {
    width: 100%;
    min-width: 0;
    flex-shrink: 0;
  }
  .file-type-columns {
    display: flex;
    justify-content: space-between;
    font-size: var(--font-caption);
    color: var(--muted);
    margin-top: 10px;
    padding: 0 2px 5px;
    border-bottom: 1px solid var(--line);
  }
  .file-type-list {
    flex: 1;
    /* Keep about three rows visible; the panel scrolls instead of squeezing the list to nothing. */
    min-height: calc(3 * 64px);
    overflow-y: auto;
    overscroll-behavior-y: contain;
    scrollbar-width: thin;
    margin: 0 -6px;
    padding: 0 6px;
  }
  ul {
    padding: 0;
    margin: 0;
    list-style: none;
    position: relative;
  }
  li {
    border-bottom: 1px solid var(--line);
    position: absolute;
    left: 0;
    right: 0;
    box-sizing: border-box;
  }
  .file-types .file-type-row {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    width: 100%;
    height: calc(100% - 4px);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    margin: 2px 0;
    padding: 6px 3px;
    background: transparent;
    text-align: left;
    box-shadow: none;
  }
  .file-types .file-type-row:hover {
    background: var(--soft);
  }
  .file-types .file-type-row.active {
    border-color: var(--accent);
    background: var(--soft);
  }
  .file-type-swatch {
    display: inline-block;
    width: 14px;
    height: 14px;
    border-radius: var(--radius-xs);
    flex-shrink: 0;
    border: 1px solid var(--swatch-edge);
  }
  .file-type-name,
  .file-type-size {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .file-type-name strong {
    font-size: var(--font-secondary);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file-type-description {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: var(--font-caption);
  }
  .file-type-name small,
  .file-type-size small {
    font-size: var(--font-caption);
    color: var(--muted);
  }
  .file-type-size {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .file-type-size strong {
    font-size: var(--font-caption);
    font-weight: 500;
  }
  .file-type-selection {
    border-top: 1px solid var(--line);
    padding-top: 10px;
    margin-top: 8px;
    flex-shrink: 0;
  }
  .file-type-selection > div {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .file-type-selection strong {
    font-size: var(--font-caption);
    overflow-wrap: anywhere;
  }
  .file-types .file-type-clear {
    width: auto;
    margin: 0 0 0 auto;
    border: 0;
    background: transparent;
    padding: 2px;
    color: var(--accent-text);
  }
  .file-type-selection p {
    font-size: var(--font-caption);
    color: var(--muted);
    margin: 5px 0 8px 21px;
  }
  .file-type-selection > small {
    display: block;
    color: var(--muted);
    font-size: var(--font-caption);
    margin-bottom: 6px;
  }
  .file-type-note {
    margin: 10px 0 0;
    font-size: var(--font-caption);
    line-height: 1.5;
    flex-shrink: 0;
  }
  .file-type-partial {
    color: var(--warning);
  }
  .file-types .file-type-error {
    color: var(--danger);
    font-size: var(--font-caption);
    overflow-wrap: anywhere;
    margin-left: 0;
  }
  @media (max-height: 700px) {
    .file-type-selection {
      padding-top: 6px;
      margin-top: 5px;
    }
    .file-type-selection p {
      margin-top: 3px;
      margin-bottom: 4px;
    }
    .file-type-note {
      margin-top: 6px;
    }
  }
</style>
