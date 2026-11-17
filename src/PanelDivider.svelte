<script lang="ts">
  import { untrack } from "svelte";
  import { DEFAULT_MAP_SHARE } from "./explorerPresentation";
  let {
    value,
    onChange,
  }: { value: number; onChange: (value: number) => void } = $props();
  let divider: HTMLDivElement;
  let available = $state(500);
  let mapMinimum = $state(150);
  let listMinimum = $state(200);
  let dragging = $state(false);
  let gesture: { id: number; x: number; share: number; width: number } | null =
    null;
  const minimum = $derived(
    Math.min(
      Math.max(0.5, mapMinimum / (mapMinimum + listMinimum)),
      mapMinimum / available,
    ),
  );
  const maximum = $derived(Math.max(minimum, 1 - listMinimum / available));

  function measureBounds(parent: HTMLElement, width: number) {
    const style = getComputedStyle(parent);
    const map = parseFloat(style.getPropertyValue("--map-min-width"));
    const list = parseFloat(style.getPropertyValue("--list-min-width"));
    mapMinimum = Number.isFinite(map) && map > 0 ? map : 150;
    listMinimum = Number.isFinite(list) && list > 0 ? list : 200;
    available = width;
  }

  function update(next: number) {
    onChange(Math.max(minimum, Math.min(maximum, next)));
  }
  $effect(() => {
    const parent = divider?.parentElement;
    if (!parent || typeof ResizeObserver === "undefined") return;
    const root = document.documentElement.style;
    const measure = () => {
      if (getComputedStyle(divider).display === "none") {
        root.removeProperty("--panel-grip-y");
        return;
      }
      // Lets the locations sidebar grip sit at the same height as this one.
      // shortcut: not refreshed when main scrolls in short windows; listen to scroll if that matters.
      const rect = divider.getBoundingClientRect();
      root.setProperty("--panel-grip-y", `${rect.top + rect.height / 2}px`);
      const width = parent.clientWidth - divider.offsetWidth;
      if (width <= 0) return;
      measureBounds(parent, width);
      const next = Math.max(minimum, Math.min(maximum, value));
      if (next !== value) onChange(next);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(parent);
    untrack(measure);
    return () => {
      observer.disconnect();
      root.removeProperty("--panel-grip-y");
    };
  });
  function start(event: PointerEvent) {
    if (event.button !== 0 || gesture) return;
    const panel = divider.parentElement?.querySelector(".visual");
    if (!panel) return;
    event.preventDefault();
    divider.focus({ preventScroll: true });
    const width = divider.parentElement!.clientWidth - divider.offsetWidth;
    if (width <= 0) return;
    measureBounds(divider.parentElement!, width);
    gesture = {
      id: event.pointerId,
      x: event.clientX,
      share: panel.getBoundingClientRect().width / width,
      width,
    };
    divider.setPointerCapture(event.pointerId);
    dragging = true;
  }
  function move(event: PointerEvent) {
    if (gesture?.id !== event.pointerId) return;
    update(gesture.share + (event.clientX - gesture.x) / gesture.width);
  }
  function finish(event: PointerEvent) {
    if (gesture?.id !== event.pointerId) return;
    gesture = null;
    dragging = false;
    if (divider.hasPointerCapture(event.pointerId))
      divider.releasePointerCapture(event.pointerId);
  }
  function keyboard(event: KeyboardEvent) {
    const step = event.shiftKey ? 0.1 : 0.02;
    const parent = divider.parentElement;
    const width = parent ? parent.clientWidth - divider.offsetWidth : 0;
    if (parent && width > 0) measureBounds(parent, width);
    // Track sizing can change before ResizeObserver delivers the new bounds.
    const panel = parent?.querySelector(".visual");
    const current =
      panel && width > 0 ? panel.getBoundingClientRect().width / width : value;
    const next =
      event.key === "ArrowLeft"
        ? current - step
        : event.key === "ArrowRight"
          ? current + step
          : event.key === "Home"
            ? minimum
            : event.key === "End"
              ? maximum
              : event.key === "Enter"
                ? DEFAULT_MAP_SHARE
                : null;
    if (next === null) return;
    event.preventDefault();
    update(next);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (A focusable ARIA window splitter has separator semantics and supports pointer and keyboard resizing.) -->
<div
  bind:this={divider}
  class="panel-divider"
  class:dragging
  role="separator"
  tabindex="0"
  aria-label="Resize storage map and file list"
  aria-orientation="vertical"
  aria-controls="storage-map-panel file-list-column"
  aria-valuemin={Math.round(minimum * 100)}
  aria-valuemax={Math.round(maximum * 100)}
  aria-valuenow={Math.round(value * 100)}
  aria-valuetext={`${Math.round(value * 100)}% map, ${Math.round((1 - value) * 100)}% file list`}
  title="Drag to resize · Arrow keys to adjust · Double-click or Enter to reset"
  onpointerdown={start}
  onpointermove={move}
  onpointerup={finish}
  onpointercancel={finish}
  onlostpointercapture={finish}
  onkeydown={keyboard}
  ondblclick={() => update(DEFAULT_MAP_SHARE)}
></div>

<style>
  .panel-divider {
    grid-column: 2;
    grid-row: 1;
    cursor: col-resize;
    touch-action: none;
    user-select: none;
    position: relative;
    border-radius: var(--radius-xs);
    z-index: 1;
  }
  .panel-divider::after {
    content: "";
    position: absolute;
    width: 4px;
    height: 52px;
    border-radius: var(--radius-xs);
    background: var(--muted);
    opacity: 0.75;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
  }
  .panel-divider:hover::after,
  .panel-divider:focus-visible::after,
  .panel-divider.dragging::after {
    background: var(--accent);
    opacity: 1;
  }
  /* The accent grip marks focus; a full-height outline flashes on every grab. */
  .panel-divider:focus-visible {
    outline: none;
  }
  @media (max-width: 800px) {
    .panel-divider {
      display: none;
    }
  }
</style>
