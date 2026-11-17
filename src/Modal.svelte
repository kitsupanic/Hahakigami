<script lang="ts">
  import type { Snippet } from "svelte";
  import { dialogFocus } from "./dialogFocus";

  // Shared modal surface: backdrop, focus trap, Escape and focus return.
  // The content names itself with an element whose id is `labelledby`.
  let {
    labelledby,
    describedby,
    opener = null,
    initialFocus,
    class: className = "",
    onClose,
    children,
  }: {
    labelledby: string;
    describedby?: string;
    opener?: HTMLElement | null;
    initialFocus?: string;
    class?: string;
    onClose: () => void;
    children: Snippet;
  } = $props();
</script>

<div class="modal-backdrop">
  <div
    class="modal {className}"
    role="dialog"
    aria-modal="true"
    aria-labelledby={labelledby}
    aria-describedby={describedby}
    tabindex="-1"
    use:dialogFocus={{ opener, initialFocus, onClose }}
  >
    {@render children()}
  </div>
</div>
