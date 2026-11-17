<script lang="ts">
  // Width in px; null keeps the responsive CSS default.
  let {
    value,
    onChange,
  }: { value: number | null; onChange: (value: number | null) => void } =
    $props();
  const MIN = 180;
  let divider: HTMLDivElement;
  let dragging = $state(false);
  let gesture: { id: number; x: number; width: number } | null = null;
  const maximum = () => Math.max(MIN, Math.min(480, window.innerWidth * 0.4));
  const current = () =>
    value ?? divider.previousElementSibling?.getBoundingClientRect().width ?? MIN;
  function update(next: number) {
    onChange(Math.round(Math.max(MIN, Math.min(maximum(), next))));
  }
  function start(event: PointerEvent) {
    if (event.button !== 0 || gesture) return;
    event.preventDefault();
    divider.focus({ preventScroll: true });
    gesture = { id: event.pointerId, x: event.clientX, width: current() };
    divider.setPointerCapture(event.pointerId);
    dragging = true;
  }
  function move(event: PointerEvent) {
    if (gesture?.id !== event.pointerId) return;
    update(gesture.width + event.clientX - gesture.x);
  }
  function finish(event: PointerEvent) {
    if (gesture?.id !== event.pointerId) return;
    gesture = null;
    dragging = false;
    if (divider.hasPointerCapture(event.pointerId))
      divider.releasePointerCapture(event.pointerId);
  }
  function keyboard(event: KeyboardEvent) {
    const step = event.shiftKey ? 40 : 10;
    if (event.key === "Enter") onChange(null);
    else if (event.key === "ArrowLeft") update(current() - step);
    else if (event.key === "ArrowRight") update(current() + step);
    else if (event.key === "Home") update(MIN);
    else if (event.key === "End") update(maximum());
    else return;
    event.preventDefault();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (A focusable ARIA window splitter has separator semantics and supports pointer and keyboard resizing.) -->
<div
  bind:this={divider}
  class="sidebar-divider"
  class:dragging
  role="separator"
  tabindex="0"
  aria-label="Resize sidebar"
  aria-orientation="vertical"
  aria-controls="locations-sidebar"
  aria-valuemin={MIN}
  aria-valuenow={value ?? undefined}
  title="Drag to resize · Arrow keys to adjust · Double-click or Enter to reset"
  onpointerdown={start}
  onpointermove={move}
  onpointerup={finish}
  onpointercancel={finish}
  onlostpointercapture={finish}
  onkeydown={keyboard}
  ondblclick={() => onChange(null)}
></div>

<style>
  .sidebar-divider {
    position: fixed;
    top: 0;
    bottom: 0;
    /* Mirrors the panel divider's 8px gutter so both grips sit 2px off their edge. */
    left: var(--sidebar-width);
    width: 8px;
    cursor: col-resize;
    touch-action: none;
    user-select: none;
    z-index: 5;
  }
  .sidebar-divider::after {
    content: "";
    position: absolute;
    width: 4px;
    height: 52px;
    border-radius: var(--radius-xs);
    background: var(--muted);
    opacity: 0.75;
    left: 50%;
    top: var(--panel-grip-y, 50%);
    transform: translate(-50%, -50%);
  }
  .sidebar-divider:hover::after,
  .sidebar-divider:focus-visible::after,
  .sidebar-divider.dragging::after {
    background: var(--accent);
    opacity: 1;
  }
  /* The accent grip marks focus; a full-height outline flashes on every grab. */
  .sidebar-divider:focus-visible {
    outline: none;
  }
</style>
