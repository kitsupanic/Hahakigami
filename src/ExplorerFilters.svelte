<script lang="ts">
  import {
    emptyFilter,
    type ExplorerFilter,
    type FilterCategory,
  } from "./filterTypes";
  import "./filter.css";
  const units = { B: 1, KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3, TB: 1024 ** 4 };
  const presets: Record<string, number> = {
    "100mb": 100 * units.MB,
    "1gb": units.GB,
    "10gb": 10 * units.GB,
  };
  function preset(filter: ExplorerFilter) {
    if (filter.minBytes === null && filter.maxBytes === null) return "any";
    return (
      Object.keys(presets).find(
        (key) => filter.maxBytes === null && filter.minBytes === presets[key],
      ) ?? "custom"
    );
  }
  interface ExplorerFiltersProps {
    value: ExplorerFilter;
    disabled?: boolean;
    onChange: (value: ExplorerFilter) => void;
  }
  import { untrack } from "svelte";
  let { value, disabled = false, onChange }: ExplorerFiltersProps = $props();
  let sizePreset = $state.raw(untrack(() => preset(value)));
  // Remounts show the largest unit that holds the applied bytes exactly, else the largest one they reach, else GB.
  let unit: keyof typeof units = $state.raw(
    untrack(() => {
      const keys = ["TB", "GB", "MB", "KB", "B"] as const;
      const largest = Math.max(value.minBytes ?? 0, value.maxBytes ?? 0);
      if (!largest) return "GB";
      return (
        keys.find(
          (key) =>
            key !== "B" &&
            [value.minBytes, value.maxBytes].every(
              (bytes) => !bytes || bytes % units[key] === 0,
            ),
        ) ?? keys.find((key) => units[key] <= largest)!
      );
    }),
  );
  // The shortest decimal that converts back to `bytes`, so 1.1 GB never reads 1.0999….
  function text(bytes: number) {
    const exact = bytes / units[unit];
    for (let digits = 0; digits <= 15; digits++) {
      const short = String(+exact.toFixed(digits));
      if (Math.round(Number(short) * units[unit]) === bytes) return short;
    }
    return String(exact);
  }
  let minimum = $state.raw(
    untrack(() => (value.minBytes === null ? "" : text(value.minBytes))),
  );
  let maximum = $state.raw(
    untrack(() => (value.maxBytes === null ? "" : text(value.maxBytes))),
  );
  let error = $state.raw("");
  let appliedMinimum = $derived(value.minBytes);
  let appliedMaximum = $derived(value.maxBytes);
  // Keep the typed text when it already converts to the applied bytes, so 1.1 does not become 1.0999….
  const shown = (draft: string, bytes: number | null) =>
    bytes === null
      ? ""
      : draft.trim() && Math.round(Number(draft) * units[unit]) === bytes
        ? draft
        : text(bytes);
  $effect(() => {
    appliedMinimum;
    appliedMaximum;
    untrack(() => {
      sizePreset = preset(value);
      minimum = shown(minimum, value.minBytes);
      maximum = shown(maximum, value.maxBytes);
      error = "";
    });
  });
  function apply() {
    const convert = (text: string) => {
      if (!text.trim()) return null;
      const number = Number(text);
      const result = Math.round(number * units[unit]);
      if (
        !Number.isFinite(number) ||
        number < 0 ||
        !Number.isSafeInteger(result) ||
        (number > 0 && result === 0)
      )
        throw new Error(
          "Enter nonnegative sizes of at least 1 byte within the supported range.",
        );
      return result;
    };
    try {
      const minBytes = convert(minimum),
        maxBytes = convert(maximum);
      if (minBytes !== null && maxBytes !== null && minBytes > maxBytes)
        throw new Error("Minimum size must not exceed maximum size.");
      error = "";
      onChange({ ...value, minBytes, maxBytes });
    } catch (failure) {
      error = (failure as Error).message;
    }
  }
</script>

<div class="explorer-filters" role="group" aria-label="Explorer filters">
  <label
    >File category<select
      aria-label="File category"
      value={value.category}
      {disabled}
      onchange={(event) =>
        onChange({
          ...value,
          category: event.currentTarget.value as FilterCategory,
        })}
      ><option value="all">All entries</option><option value="images"
        >Images</option
      ><option value="videos">Videos</option><option value="audio">Audio</option
      ><option value="documents">Documents</option><option value="archives"
        >Archives</option
      ><option value="applications">Applications</option><option value="other"
        >Other files</option
      ></select
    ></label
  ><label
    >Size range<select
      aria-label="Size range"
      value={sizePreset}
      {disabled}
      onchange={(event) => {
        const selected = event.currentTarget.value;
        sizePreset = selected;
        error = "";
        if (selected === "custom") return;
        onChange({
          ...value,
          minBytes: selected === "any" ? null : presets[selected],
          maxBytes: null,
        });
      }}
      ><option value="any">Any size</option><option value="100mb"
        >100 MB+</option
      ><option value="1gb">1 GB+</option><option value="10gb">10 GB+</option
      ><option value="custom">Custom range</option></select
    ></label
  >{#if sizePreset === "custom"}<div class="custom-size-range">
      <label
        >Minimum<input
          aria-label="Minimum size"
          inputmode="decimal"
          placeholder="No minimum"
          value={minimum}
          {disabled}
          oninput={(event) => (minimum = event.currentTarget.value)}
        /></label
      ><label
        >Maximum<input
          aria-label="Maximum size"
          inputmode="decimal"
          placeholder="No maximum"
          value={maximum}
          {disabled}
          oninput={(event) => (maximum = event.currentTarget.value)}
        /></label
      ><label
        >Unit<select
          aria-label="Size unit"
          value={unit}
          {disabled}
          onchange={(event) =>
            (unit = event.currentTarget.value as keyof typeof units)}
          >{#each Object.keys(units) as key}<option>{key}</option
            >{/each}</select
        ></label
      ><button {disabled} onclick={apply}>Apply range</button>
    </div>{/if}<button
    {disabled}
    onclick={() => {
      sizePreset = "any";
      minimum = "";
      maximum = "";
      unit = "GB";
      error = "";
      onChange({ ...emptyFilter });
    }}>Clear filters</button
  >{#if error}<p role="alert" class="filter-error">
      {error} Previous applied range is unchanged.
    </p>{/if}<small class="filter-note"
    >Sizes count 1 KB as 1,024 bytes and use the size you picked. Categories go by file name and extension.</small
  >
</div>
