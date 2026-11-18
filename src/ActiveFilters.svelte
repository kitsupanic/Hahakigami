<script lang="ts">
  import {
    categoryLabels,
    sizeFilterDescription,
    sizeFilterLabel,
  } from "./activeFilters";
  import {
    emptyFilter,
    filterActive,
    type ExplorerFilter,
  } from "./filterTypes";
  import { fileTypeLabel } from "./fileTypes";
  import "./filter.css";

  interface ActiveFiltersProps {
    value: ExplorerFilter;
    disabled?: boolean;
    onChange: (value: ExplorerFilter) => void;
  }
  let {
    value,
    disabled = false,
    onChange,
  }: ActiveFiltersProps = $props();
  let sizeLabel = $derived(sizeFilterLabel(value));
</script>

{#if filterActive(value)}
  <div class="active-filters" role="group" aria-label="Active filters">
    {#if value.extension != null}
      <button
        class="active-filter-chip"
        {disabled}
        aria-label={`Remove file type filter: ${fileTypeLabel(value.extension)}`}
        title="Exact file extension within the chosen file type scope"
        onclick={() => {
          const { extension, scopeId, ...rest } = value;
          onChange(rest);
        }}
        ><span>{fileTypeLabel(value.extension)}</span><span aria-hidden="true"
          >×</span
        ></button
      >
    {/if}
    {#if value.category !== "all"}
      <button
        class="active-filter-chip"
        {disabled}
        aria-label={`Remove category filter: ${categoryLabels[value.category]}`}
        onclick={() => onChange({ ...value, category: "all" })}
        ><span>{categoryLabels[value.category]}</span><span aria-hidden="true"
          >×</span
        ></button
      >
    {/if}
    {#if sizeLabel}
      <button
        class="active-filter-chip"
        {disabled}
        aria-label={`Remove size filter: ${sizeLabel}`}
        title={sizeFilterDescription(value)}
        onclick={() => onChange({ ...value, minBytes: null, maxBytes: null })}
        ><span>{sizeLabel}</span><span aria-hidden="true">×</span></button
      >
    {/if}
    <button
      class="active-filter-clear"
      {disabled}
      onclick={() => onChange({ ...emptyFilter })}>Clear filters</button
    >
  </div>
{/if}
