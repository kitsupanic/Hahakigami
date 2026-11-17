import {
  createScanSlots,
  type ScanPort,
  type ScanSlotsHooks,
} from "./scanSlots.svelte";

/** createScanSlots outside a component: its effect needs a root of its own. */
export function rootedScanSlots(port: ScanPort, hooks: ScanSlotsHooks) {
  let scans!: ReturnType<typeof createScanSlots>;
  const dispose = $effect.root(() => {
    scans = createScanSlots(port, hooks);
  });
  return { scans, dispose };
}
