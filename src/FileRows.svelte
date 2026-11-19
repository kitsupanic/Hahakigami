<script lang="ts">
  import { bytes, type NodeView, type Page } from "./types";
  import Icon from "./Icon.svelte";
  import { latestOnly } from "./latestQuery";
  import { normalizePath, parentPath } from "./branchColors";
  import { createQueueMarker, type QueuePath } from "./queueMarkers";
  import { compactBytes, usageColor } from "./uiFormat";
  import {
    createHoverTarget,
    findHoverMatch,
    type HoverTarget,
  } from "./hoverContext";
  const ROW_HEIGHT = 36,
    OVERSCAN = 5;
  type Branch = Page & { loading?: boolean; error?: string };
  type Row = { node: NodeView; depth: number; more?: boolean };
  import { onDestroy, onMount, tick, untrack } from "svelte";
  let {
    items,
    selection,
    selectionPath = null,
    scanBytes = null,
    running,
    revision,
    onLoadChildren,
    onSelect,
    onOpen,
    onPreview,
    keyboardBlocked = false,
    showParentPaths = false,
    colorForNode = (node: NodeView) => usageColor(node.id),
    queuedPaths = [],
    onScrollbarWidthChange,
    hovered = null,
    onHover,
    restoreScroll = null,
    scrollLoading = false,
    onScrollChange,
    onScrollRestored,
  }: {
    items: NodeView[];
    selection: number[];
    /** Path of a single selection, so a row inside collapsed folders can be revealed. */
    selectionPath?: string | null;
    scanBytes?: number | null;
    running: boolean;
    revision?: string;
    onLoadChildren?: (nodeId: number) => Promise<Page>;
    onSelect: (n: NodeView, multi: boolean) => void;
    onOpen: (n: NodeView) => void;
    onPreview?: (n: NodeView) => boolean;
    keyboardBlocked?: boolean;
    showParentPaths?: boolean;
    colorForNode?: (node: NodeView) => string;
    queuedPaths?: readonly QueuePath[];
    onScrollbarWidthChange?: (width: number) => void;
    hovered?: HoverTarget | null;
    onHover?: (target: HoverTarget | null) => void;
    restoreScroll?: { id: number; top: number } | null;
    scrollLoading?: boolean;
    onScrollChange?: (top: number) => void;
    onScrollRestored?: (id: number) => void;
  } = $props();
  const rowHeight = $derived(showParentPaths ? 60 : ROW_HEIGHT);
  // The root has no parent to show.
  function parentDirectory(path: string) {
    const normalized = normalizePath(path);
    const parent = parentPath(normalized);
    return parent === normalized ? "" : parent;
  }
  let queueMarker = $derived(createQueueMarker(queuedPaths));
  let scroll = $state.raw(0);
  let viewHeight = $state.raw(230);
  let branches: Record<number, Branch> = $state.raw({});
  let container: HTMLDivElement | undefined;
  let focusedId = $state<number | undefined>();
  let selectionAnchor: number | undefined;
  let focusRequest = 0;
  onMount(() => {
    const element = container!;
    let scrollbarWidth: number | undefined;
    const measure = () => {
      if (element.clientHeight > 0) viewHeight = element.clientHeight;
      const width = Math.max(0, element.offsetWidth - element.clientWidth);
      if (width !== scrollbarWidth) {
        scrollbarWidth = width;
        onScrollbarWidthChange?.(width);
      }
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  });
  let generation = 0;
  let expanded = new Set<number>();
  let requests = new Map<number, number>();
  let pending = new Map<number, Promise<void>>();
  const queryBranches = latestOnly();
  function load(id: number) {
    const promise = fetchBranch(id);
    pending.set(id, promise);
    return promise;
  }
  async function fetchBranch(id: number) {
    const current = generation;
    const request = (requests.get(id) || 0) + 1;
    requests.set(id, request);
    let branch: Branch;
    try {
      branch = await onLoadChildren!(id);
    } catch (e) {
      branch = { items: [], total: 0, error: String(e) };
    }
    if (
      current === generation &&
      requests.get(id) === request &&
      expanded.has(id)
    )
      branches = ((old) => ({ ...old, [id]: branch }))(branches);
  }
  onDestroy(() => {
    generation++;
    focusRequest++;
    restoreGeneration++;
    onHover?.(null);
  });
  $effect(() => {
    revision;
    untrack(() => {
      queryBranches(() => Promise.all([...expanded].map(load)).then(() => {}));
    });
  });
  let firstId = $derived(items[0]?.id);
  let restoreGeneration = 0;
  let restoredId: number | undefined;
  $effect(() => {
    const request = restoreScroll;
    const loading = scrollLoading;
    firstId;
    const element = container;
    if (!request || loading || !element || restoredId === request.id) return;
    const current = ++restoreGeneration;
    void tick().then(() => {
      if (
        current !== restoreGeneration ||
        restoreScroll !== request ||
        scrollLoading ||
        container !== element
      )
        return;
      const maximum = Math.max(0, element.scrollHeight - element.clientHeight);
      element.scrollTop = Math.min(maximum, Math.max(0, request.top));
      scroll = element.scrollTop;
      restoredId = request.id;
      onScrollChange?.(scroll);
      onScrollRestored?.(request.id);
    });
    return () => {
      restoreGeneration++;
    };
  });
  function toggle(node: NodeView) {
    if (!onLoadChildren || node.excluded) return;
    if (expanded.has(node.id)) {
      expanded.delete(node.id);
      branches = ((old) => {
        const next = { ...old };
        delete next[node.id];
        return next;
      })(branches);
    } else {
      expanded.add(node.id);
      branches = ((old) => ({
        ...old,
        [node.id]: { items: [], total: node.childrenCount, loading: true },
      }))(branches);
      return load(node.id);
    }
  }
  let rows = $derived.by(() => {
    const result: Row[] = [];
    function append(nodes: NodeView[], depth: number) {
      for (const node of nodes) {
        result.push({ node, depth });
        const branch = branches[node.id];
        if (branch && !node.excluded) {
          append(branch.items, depth + 1);
          if (
            branch.loading ||
            branch.error ||
            branch.total > branch.items.length
          )
            result.push({ node, depth: depth + 1, more: true });
        }
      }
    }
    append(items, 0);
    return result;
  });
  // A selection made on the map brings its row into view, opening the folders
  // above it when they are collapsed. Clicks and arrow keys select rows that are
  // already visible, so this does nothing for them.
  let revealedKey = "";
  let revealRequest = 0;
  $effect(() => {
    const key = selection.length === 1 ? String(selection[0]) : "";
    if (key === revealedKey) return;
    revealedKey = key;
    revealRequest++;
    if (!key) return;
    const id = selection[0];
    const path = selectionPath;
    untrack(() => void reveal(id, path));
  });
  const isAncestor = (parent: string, path: string) =>
    path.startsWith(parent.endsWith("/") ? parent : `${parent}/`);
  async function reveal(id: number, path: string | null) {
    const request = revealRequest;
    // Each pass opens one folder; the bound only guards against a cycle in bad data.
    for (let depth = 0; depth < 256 && request === revealRequest; depth++) {
      const index = rows.findIndex((row) => !row.more && row.node.id === id);
      if (index >= 0) return scrollToRow(index);
      const parent = path
        ? rows
            .filter(
              (row) =>
                !row.more &&
                row.node.kind === "directory" &&
                isAncestor(row.node.path, path),
            )
            .at(-1)?.node
        : undefined;
      if (!parent || !onLoadChildren || parent.excluded) return;
      // Already open and loaded without the row: it is past the loaded page.
      if (expanded.has(parent.id) && !branches[parent.id]?.loading) return;
      await (expanded.has(parent.id) ? pending.get(parent.id) : toggle(parent));
      await tick();
    }
  }
  function scrollToRow(index: number) {
    const element = container;
    if (!element) return;
    const top = index * rowHeight;
    if (top >= scroll && top + rowHeight <= scroll + viewHeight) return;
    const target = Math.max(0, top - (viewHeight - rowHeight) / 2);
    const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    if (!still && typeof element.scrollTo === "function") {
      // The scroll handler keeps the virtual window and the parent in step as it glides.
      element.scrollTo({ top: target, behavior: "smooth" });
      return;
    }
    element.scrollTop = target;
    scroll = element.scrollTop;
    onScrollChange?.(scroll);
  }
  $effect.pre(() => {
    rows;
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && container?.contains(focused)) {
      // Moving a keyed DOM node can blur it in engines without moveBefore.
      void tick().then(() => {
        if (
          document.activeElement === document.body &&
          container?.contains(focused)
        )
          focused.focus({ preventScroll: true });
      });
    }
  });
  let maxSize = $derived(Math.max(1, ...items.map((n) => n.allocatedBytes)));
  let start = $derived(Math.max(0, Math.floor(scroll / rowHeight) - OVERSCAN));
  let end = $derived(
    Math.min(
      rows.length,
      Math.ceil((scroll + viewHeight) / rowHeight) + OVERSCAN,
    ),
  );
  const rowIndex = $derived(
    new Map(rows.filter((row) => !row.more).map(({ node }) => [node.id, node])),
  );
  const visibleIds = $derived(
    new Set(
      rows
        .slice(
          Math.floor(scroll / rowHeight),
          Math.ceil((scroll + viewHeight) / rowHeight),
        )
        .filter((row) => !row.more)
        .map(({ node }) => node.id),
    ),
  );
  const hoveredId = $derived(findHoverMatch(hovered, visibleIds));
  const tabStop = $derived(
    rows.slice(start, end).find((row) => !row.more && row.node.id === focusedId)
      ?.node.id ??
      rows.slice(Math.floor(scroll / rowHeight), end).find((row) => !row.more)
        ?.node.id,
  );
  async function browse(event: KeyboardEvent, node: NodeView) {
    if (event.altKey || !container) return false;
    const navigable = rows.flatMap((row, index) =>
      row.more ? [] : [{ node: row.node, index }],
    );
    const current = navigable.findIndex((row) => row.node.id === node.id);
    const page = Math.max(1, Math.floor(viewHeight / rowHeight));
    const next =
      event.key === "ArrowDown"
        ? current + 1
        : event.key === "ArrowUp"
          ? current - 1
          : event.key === "PageDown"
            ? current + page
            : event.key === "PageUp"
              ? current - page
              : event.key === "Home"
                ? 0
                : event.key === "End"
                  ? navigable.length - 1
                  : null;
    if (next === null || current < 0) return false;
    event.preventDefault();
    const target = Math.max(0, Math.min(navigable.length - 1, next));
    const destination = navigable[target];
    if (event.shiftKey) {
      let anchor = navigable.findIndex(
        (row) => row.node.id === selectionAnchor,
      );
      if (anchor < 0) {
        anchor = current;
        selectionAnchor = node.id;
      }
      const range = navigable.slice(
        Math.min(anchor, target),
        Math.max(anchor, target) + 1,
      );
      range.forEach((row, index) => onSelect(row.node, index > 0));
    } else if (!event.metaKey && !event.ctrlKey) {
      selectionAnchor = destination.node.id;
      onSelect(destination.node, false);
    }
    focusedId = destination.node.id;
    const element = container;
    const top = destination.index * rowHeight;
    if (top < scroll) scroll = top;
    else if (top + rowHeight > scroll + viewHeight)
      scroll = top + rowHeight - viewHeight;
    element.scrollTop = scroll;
    onScrollChange?.(scroll);
    onHover?.(null);
    const previous = document.activeElement;
    const snapshot = rows;
    const request = ++focusRequest;
    await tick();
    if (
      request !== focusRequest ||
      rows !== snapshot ||
      container !== element ||
      !element.isConnected ||
      element.closest("[hidden]") ||
      (document.activeElement !== previous &&
        document.activeElement !== document.body)
    )
      return true;
    element
      .querySelector<HTMLElement>(`[data-node-id="${destination.node.id}"]`)
      ?.focus({ preventScroll: true });
    return true;
  }
</script>

<div
  bind:this={container}
  class="rows"
  class:has-parent-paths={showParentPaths}
  role="tree"
  aria-label="Folders and files"
  aria-multiselectable="true"
  onscroll={(e) => {
    scroll = e.currentTarget.scrollTop;
    onScrollChange?.(scroll);
    onHover?.(null);
  }}
>
  <div
    style:height={rows.length * rowHeight + "px"}
    style:position={"relative"}
  >
    {#each rows.slice(start, end) as { node: n, depth, more }, i (`${n.id}:${more ? "more" : "node"}`)}{#if more}<div
          class="row tree-message"
          style:height={rowHeight + "px"}
          style:position={"absolute"}
          style:top={(start + i) * rowHeight + "px"}
          style:left={0 + "px"}
        >
          <button
            style:margin-left={depth * 18 + "px"}
            onclick={() => (branches[n.id].error ? void load(n.id) : onOpen(n))}
            disabled={branches[n.id].loading}
            >{branches[n.id].loading
              ? "Loading…"
              : branches[n.id].error
                ? "Unable to load · Retry"
                : `Open folder to see all ${branches[n.id].total.toLocaleString()} items ↗`}</button
          >
        </div>{:else}{@const queueState =
          queueMarker(n) === "contains" && n.kind !== "directory"
            ? null
            : queueMarker(n)}
        {@const parentDir = showParentPaths ? parentDirectory(n.path) : ""}
        <div
          role="treeitem"
          data-kind={n.kind}
          data-node-id={n.id}
          data-queue-state={queueState || undefined}
          aria-level={depth + 1}
          aria-selected={selection.includes(n.id)}
          aria-expanded={n.kind === "directory" && !n.excluded && onLoadChildren
            ? !!branches[n.id]
            : undefined}
          tabindex={tabStop === n.id ? 0 : -1}
          class={`row ${selection.includes(n.id) ? "selected" : ""} ${queueState === "queued" ? "queued" : ""}`}
          class:linked-hover={hoveredId === n.id}
          style:height={rowHeight + "px"}
          style:position={"absolute"}
          style:top={(start + i) * rowHeight + "px"}
          style:left={0 + "px"}
          onmousemove={() => onHover?.(createHoverTarget(n, rowIndex))}
          onmouseleave={() => onHover?.(null)}
          onfocus={() => (focusedId = n.id)}
          onclick={(e) => {
            focusedId = n.id;
            selectionAnchor = n.id;
            e.currentTarget.focus({ preventScroll: true });
            onSelect(n, e.metaKey || e.ctrlKey);
          }}
          ondblclick={() => (n.excluded ? onSelect(n, false) : onOpen(n))}
          onkeydown={(e) => {
            if (
              keyboardBlocked ||
              e.target !== e.currentTarget ||
              e.currentTarget.closest(
                '[role="dialog"], [aria-modal="true"], [contenteditable="true"]',
              )
            )
              return;
            if (
              [
                "ArrowDown",
                "ArrowUp",
                "Home",
                "End",
                "PageDown",
                "PageUp",
              ].includes(e.key)
            ) {
              void browse(e, n);
              return;
            }
            if (e.key === "Enter") {
              if (n.excluded) onSelect(n, e.metaKey || e.ctrlKey);
              else onOpen(n);
              e.preventDefault();
            } else if (e.key === " ") {
              if (e.altKey || e.shiftKey || e.repeat) return;
              if (e.ctrlKey || e.metaKey) {
                selectionAnchor = n.id;
                onSelect(n, true);
                e.preventDefault();
                return;
              }
              if (n.kind === "file" && !n.excluded && onPreview?.(n)) {
                e.preventDefault();
                return;
              }
              selectionAnchor = n.id;
              onSelect(n, false);
              e.preventDefault();
            } else if (
              ((e.key === "ArrowRight" && !branches[n.id]) ||
                (e.key === "ArrowLeft" && branches[n.id])) &&
              n.kind === "directory"
            ) {
              toggle(n);
              e.preventDefault();
            }
          }}
        >
          <span
            class="file-row-name"
            style:--branch-color={colorForNode(n)}
            style:padding-left={Math.min(depth, 20) * 18 + "px"}
            >{#if n.kind === "directory" && !n.excluded && onLoadChildren}<button
                class="disclosure"
                tabindex={-1}
                aria-label={`${branches[n.id] ? "Collapse" : "Expand"} ${n.name}`}
                onclick={(e) => {
                  e.stopPropagation();
                  toggle(n);
                }}
                ondblclick={(e) => e.stopPropagation()}
                ><span
                  style:display="inline-flex"
                  style:transform={branches[n.id] ? "rotate(90deg)" : undefined}
                  ><Icon name="chevron" size={13} /></span
                ></button
              >{:else}<span class="disclosure-space"></span>{/if}<Icon
              name={n.kind === "directory" ? "folder" : "file"}
              size={32}
            /><span class="file-label"
              ><span class="file-name" title={showParentPaths ? n.path : n.name}
                >{n.name}</span
              >{#if parentDir}<span class="file-parent-path" title={parentDir}
                  >{parentDir}</span
                >{/if}</span
            >{#if queueState}<span
                class:queue-contains={queueState === "contains"}
                class="queue-badge"
                title={`${n.name}: ${queueState === "queued" ? "queued for cleanup" : "contains items queued for cleanup"}`}
                ><Icon
                  name={queueState === "queued" ? "check" : "queue"}
                  size={12}
                />{queueState === "queued" ? "Queued" : "Queued inside"}</span
              >{/if}<em>{n.incomplete && !n.excluded ? "Scan issues" : ""}</em
            ></span
          ><span class="file-relative"
            >{#if !n.excluded && scanBytes !== null}
              {scanBytes > 0
                ? `${((n.allocatedBytes / scanBytes) * 100).toFixed(1)}%`
                : "—"}
            {:else if !n.excluded}<span class="file-share" aria-hidden="true"
                ><span
                  style:width={`${Math.min(100, Math.max(0, (n.allocatedBytes / maxSize) * 100))}%`}
                  style:background={colorForNode(n)}
                ></span></span
              >{/if}</span
          ><span
            class="file-size"
            title={n.excluded ? undefined : bytes(n.allocatedBytes)}
            >{n.excluded ? "Excluded" : compactBytes(n.allocatedBytes)}</span
          ><span class="file-contents"
            >{n.excluded
              ? "Not scanned"
              : n.kind === "directory"
                ? `${n.childrenCount.toLocaleString()} items`
                : "—"}</span
          >
        </div>{/if}{/each}
  </div>
  {#if !items.length}<p class="no-files">
      {running ? "Waiting for entries…" : "No matching files"}
    </p>{/if}
</div>

<style>
  .row.linked-hover {
    outline: 2px solid var(--warning-mark);
    outline-offset: -2px;
  }
  .row.linked-hover:not(.selected) {
    background: color-mix(in srgb, var(--warning-mark) 10%, var(--panel));
  }
</style>
