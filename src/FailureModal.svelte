<script lang="ts">
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  // A failure the user must act on: a plain-words reason, the raw error as a
  // technical detail, an optional recovery action and a way out.
  let {
    title,
    reason,
    details = "",
    actionLabel,
    actionDisabled = false,
    closeLabel = "Close",
    opener = null,
    onAction,
    onClose,
  }: {
    title: string;
    reason: string;
    details?: string;
    actionLabel?: string;
    actionDisabled?: boolean;
    closeLabel?: string;
    opener?: HTMLElement | null;
    onAction?: () => void;
    onClose: () => void;
  } = $props();
</script>

<Modal
  class="failure-modal"
  labelledby="failure-title"
  initialFocus="[data-failure-primary]"
  {opener}
  {onClose}
>
  <section class="failure-results">
    <div class="failure-heading">
      <span class="failure-icon"><Icon name="info" size={18} /></span>
      <div>
        <h2 id="failure-title">{title}</h2>
        <p class="failure-reason">{reason}</p>
      </div>
    </div>
    {#if details}
      <details class="failure-details">
        <summary>Technical details</summary>
        <p>{details}</p>
      </details>
    {/if}
    <div class="failure-actions">
      <button
        class:primary={!actionLabel}
        data-failure-primary={actionLabel ? undefined : ""}
        onclick={onClose}>{closeLabel}</button
      >
      {#if actionLabel}
        <button
          class="primary"
          data-failure-primary
          disabled={actionDisabled}
          onclick={onAction}>{actionLabel}</button
        >
      {/if}
    </div>
  </section>
</Modal>
