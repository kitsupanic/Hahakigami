<script lang="ts">
  import { onMount } from "svelte";
  import createApp from "./createApp.svelte";
  let { capture }: { capture: (app: ReturnType<typeof createApp>) => void } =
    $props();
  const app = createApp();
  onMount(() => capture(app));
</script>
