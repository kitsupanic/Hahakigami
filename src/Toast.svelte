<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { AppToast } from "./toastTypes";

  let { toast, onDismiss }: { toast: AppToast; onDismiss: () => void } =
    $props();
  const DISMISS_MS = 4000;
  let paused = $state(false);
  $effect(() => {
    if (paused) return;
    const timer = setTimeout(onDismiss, DISMISS_MS);
    return () => clearTimeout(timer);
  });
</script>

<!-- Hover and focus only pause the timer; the buttons inside are the controls. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="toast"
  onmouseover={() => (paused = true)}
  onmouseout={() => (paused = false)}
  onfocusin={() => (paused = true)}
  onfocusout={() => (paused = false)}
>
  <span class="toast-message">{toast.message}</span>
  {#if toast.action}
    <button
      class="toast-action"
      onclick={() => {
        const run = toast.action?.run;
        onDismiss();
        run?.();
      }}>{toast.action.label}</button
    >
  {/if}
  <button
    class="toast-dismiss"
    aria-label="Dismiss"
    title="Dismiss"
    onclick={onDismiss}><Icon name="close" size={15} /></button
  >
</div>
