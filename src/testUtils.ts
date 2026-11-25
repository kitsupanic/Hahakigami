import { flushSync, mount, tick, unmount, type Component } from "svelte";
import TestRoot from "./TestRoot.svelte";

/** Flush effects and asynchronous native-command continuations around an interaction. */
export async function act(callback: () => unknown | Promise<unknown>) {
  flushSync();
  await callback();
  flushSync();
  await tick();
  flushSync();
}
export interface Root {
  render<Props extends Record<string, any>>(
    component: Component<Props>,
    props?: Props & { key?: unknown },
  ): void;
  unmount(): Promise<void>;
}
export function createTestRoot(container: HTMLElement): Root {
  let instance: { update(props: Record<string, unknown>): void } | undefined;
  let current: Component<any> | undefined;
  let currentKey: unknown;
  return {
    render(component, incomingProps) {
      const props: Record<string, unknown> = incomingProps ?? {};
      if (instance && current === component && currentKey === props.key) {
        instance.update(props);
      } else {
        if (instance) void unmount(instance);
        current = component;
        currentKey = props.key;
        instance = mount(TestRoot, {
          target: container,
          props: { component, initialProps: props },
        }) as { update(props: Record<string, unknown>): void };
      }
      flushSync();
    },
    async unmount() {
      if (instance) await unmount(instance);
      instance = undefined;
      current = undefined;
      currentKey = undefined;
      flushSync();
    },
  };
}

export const performanceMapRecord: {
  current: { order: string[]; lastPaint: (() => void) | null } | null;
} = { current: null };
/** What the app answers to `get_edition` in the Full edition; the default in every App test. */
export const fullEdition = {
  productName: "Hahakigami",
  automationScans: true,
  scanTargetsRequireAllowedLocation: false,
};
