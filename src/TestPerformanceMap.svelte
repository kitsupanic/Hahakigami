<script lang="ts">
  import TestPaintMap from "./TestPaintMap.svelte";
  import type { NodeView } from "./types";
  import { performanceMapRecord } from "./testUtils";
  let { nodes, onRendered }: { nodes: NodeView[]; onRendered?: (nodes: NodeView[]) => void } = $props();
</script>
<TestPaintMap {nodes} {onRendered} record={performanceMapRecord.current!} />
