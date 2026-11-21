<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { AppToast } from "./toastTypes";

  let { toast, onDismiss }: { toast: AppToast; onDismiss: () => void } =
    $props();
  const DISMISS_MS = 4000;
  // Tracked apart so leaving with the pointer does not resume while a button keeps focus.
  let hovered = $state(false);
  let focused = $state(false);
  $effect(() => {
    if (hovered || focused) return;
    const timer = setTimeout(onDismiss, DISMISS_MS);
    return () => clearTimeout(timer);
  });
</script>

<!-- Hover and focus only pause the timer; the buttons inside are the controls. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="toast"
  {@attach (node) => {
    // A toast that replaces another under the pointer gets no mouseenter.
    if (node.matches(":hover")) hovered = true;
  }}
  onmouseenter={() => (hovered = true)}
  onmouseleave={() => (hovered = false)}
  onfocusin={() => (focused = true)}
  onfocusout={() => (focused = false)}
>
  <span class="toast-message">{toast.message}</span>
  <button
    class="toast-dismiss"
    aria-label="Dismiss"
    title="Dismiss"
    onclick={onDismiss}><Icon name="close" size={15} /></button
  >
</div>
