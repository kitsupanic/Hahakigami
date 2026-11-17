<script lang="ts">
  import { bytes, type LibraryProgress, type SavedEntry } from "./types";
  import { compactBytes } from "./uiFormat";

  interface SavedScanLoadingProps {
    entry?:
      | (Pick<SavedEntry, "name" | "rootPath"> &
          Partial<Pick<SavedEntry, "entries">>)
      | null;
    progress?: LibraryProgress | null;
    onCancel?: () => void;
    cancelling?: boolean;
  }
  let {
    entry = null,
    progress = null,
    onCancel,
    cancelling = false,
  }: SavedScanLoadingProps = $props();
  let opening = $derived(progress?.operation === "open");
  let phase = $derived(
    progress?.phase === "reading"
      ? "Reading saved results"
      : progress?.phase === "validating"
        ? "Validating saved results"
        : progress?.phase === "done"
          ? "Preparing explorer"
          : "Preparing saved results",
  );
</script>

<section class="saved-scan-loading" aria-label="Saved scan loading">
  <div class="loading-card">
    <div aria-busy="true">
      <div class="loading-heading">
        <span class="loading-spinner" aria-hidden="true"></span>
        <h1>{opening ? "Opening saved scan" : "Loading saved scans"}</h1>
      </div>
      {#if opening && entry}
        <h2>{entry.name}</h2>
        <p class="loading-path">{entry.rootPath}</p>
      {/if}
    </div>
    <p role="status" aria-live="polite">
      {cancelling
        ? "Stopping saved scan opening"
        : opening
          ? phase
          : "Preparing your saved scan library"}…
    </p>
    {#if opening && progress}
      <p class="loading-processed" title={bytes(progress.bytes)}>
        Decoded <strong>{compactBytes(progress.bytes)}</strong>
      </p>
    {/if}
    {#if opening && entry?.entries !== undefined}
      <p class="loading-processed">
        {entry.entries.toLocaleString()} recorded entries · Size varies after decoding
      </p>
    {/if}
    {#if opening && onCancel}
      <button class="loading-cancel" onclick={onCancel} disabled={cancelling}>
        {cancelling ? "Stopping…" : "Cancel opening"}
      </button>
    {/if}
    <div class="loading-workspace" aria-hidden="true">
      <div></div>
      <div></div>
      <div></div>
    </div>
    <p class="loading-note">You can view saved scans but not change them.</p>
  </div>
</section>

<style>
  .saved-scan-loading {
    display: grid;
    place-items: center;
    min-height: 0;
    height: 100%;
    padding: 32px;
    box-sizing: border-box;
    overflow: auto;
  }
  .loading-card {
    width: min(100%, 480px);
    padding: 28px;
    box-sizing: border-box;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius-lg);
  }
  .loading-heading {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  h1 {
    margin: 0;
    font-size: var(--font-title2);
    font-weight: 600;
  }
  h2 {
    margin: 24px 0 6px;
    font-size: var(--font-title3);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  p {
    margin: 16px 0 0;
    line-height: 1.5;
  }
  .loading-path {
    margin: 0;
    font-size: var(--font-secondary);
    color: var(--muted);
    overflow-wrap: anywhere;
    user-select: text;
  }
  .loading-processed {
    font-size: var(--font-secondary);
    color: var(--muted);
  }
  .loading-processed strong {
    margin-inline-start: 6px;
    font-weight: 500;
    color: inherit;
    font-variant-numeric: tabular-nums;
  }
  .loading-spinner {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    border: 2px solid var(--line);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: loading-spin 1s linear infinite;
  }
  .loading-cancel {
    margin-top: 20px;
  }
  .loading-workspace {
    display: grid;
    gap: 10px;
    margin-top: 24px;
  }
  .loading-workspace > div {
    height: 9px;
    border-radius: var(--radius-xs);
    background: var(--line);
    opacity: 0.65;
  }
  .loading-workspace > div:nth-child(2) {
    width: 75%;
  }
  .loading-workspace > div:nth-child(3) {
    width: 45%;
  }
  .loading-note {
    margin-top: 24px;
    color: var(--muted);
    font-size: var(--font-caption);
  }
  @keyframes loading-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .loading-spinner {
      animation: none;
    }
  }
  @media (max-width: 600px) {
    .saved-scan-loading {
      padding: 16px;
    }
    .loading-card {
      padding: 20px;
    }
  }
</style>
