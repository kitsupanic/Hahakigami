<script lang="ts">
  import type { NodeView } from "./types";
  let { nodes, onRendered, record }: {
    nodes: NodeView[];
    onRendered?: (nodes: NodeView[]) => void;
    record: { order: string[]; lastPaint: (() => void) | null };
  } = $props();
  $effect(() => {
    // Capture the committed nodes so late callbacks remain attributable to their map.
    const committedNodes = nodes;
    const committedOnRendered = onRendered;
    record.lastPaint = () => {
      record.order.push(`draw:${committedNodes[0]?.name}`);
      committedOnRendered?.(committedNodes);
    };
  });
</script>
<button onclick={() => record.lastPaint?.()}>Paint current map</button>
