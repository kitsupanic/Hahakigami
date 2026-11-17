<script lang="ts">
  import type { NodeView } from "./types";
  let {
    nodes = [],
    onSelect,
  }: {
    nodes?: NodeView[];
    onSelect?: (node: NodeView, multi: boolean) => void;
  } = $props();
</script>

<div>
  Map canvas
  {#if nodes.length}
    <button class="test-map-select" onclick={() => onSelect?.(nodes[0], false)}
      >Select first map item</button
    >
  {/if}
</div>
