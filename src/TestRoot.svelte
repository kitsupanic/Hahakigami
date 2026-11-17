<script lang="ts">
  import { untrack, type Component as SvelteComponent } from "svelte";
  let { component: CurrentComponent, initialProps = {} }: { component: SvelteComponent<any>; initialProps?: Record<string, unknown> } = $props();
  let currentProps = $state.raw(untrack(() => initialProps));
  export function update(props: Record<string, unknown>) { currentProps = props; }
</script>
<CurrentComponent {...currentProps} />
