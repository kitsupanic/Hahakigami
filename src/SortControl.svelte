<script lang="ts">
  import "./sort.css";
  interface SortControlProps {
    ready: boolean;
    saving: boolean;
    warning: string | null;
    onRetry: () => void;
  }
  let { ready, saving, warning, onRetry }: SortControlProps = $props();
</script>

{#if !ready || saving || warning}<div class="sort-control">
  {#if !ready || saving}<span role="status"
      >{saving ? "Saving sort order…" : "Loading saved sort order…"}</span
    >{/if}{#if warning}<div role="alert" class="sort-warning">
      <span>{warning}</span><button
        disabled={!ready || saving}
        onclick={onRetry}>Save current order</button
      >
    </div>{/if}
</div>{/if}
