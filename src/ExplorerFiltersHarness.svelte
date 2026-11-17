<script lang="ts">
  import ExplorerFilters from "./ExplorerFilters.svelte";
  import { emptyFilter, type ExplorerFilter } from "./filterTypes";
  let {
    disabled = false,
    applied,
  }: { disabled?: boolean; applied: (value: ExplorerFilter) => void } =
    $props();
  let value = $state.raw<ExplorerFilter>({ ...emptyFilter });
</script>

<ExplorerFilters
  {value}
  {disabled}
  onChange={(next) => {
    value = next;
    applied(next);
  }}
/>
