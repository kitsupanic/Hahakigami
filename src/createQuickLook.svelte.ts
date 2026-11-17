import { onMount } from "svelte";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { QuickLookStatus } from "./quickLookTypes";
const closed: QuickLookStatus = {
  open: false,
  scanId: null,
  nodeId: null,
  revision: 0,
};
type Identity = { scanId: string; nodeId: number };
export default function createQuickLook() {
  let status = $state.raw<QuickLookStatus>(closed);
  const setStatus = (value: QuickLookStatus) => {
    status = value;
  };
  let pending = $state(false);
  const setPending = (value: boolean) => {
    pending = value;
  };
  let error = $state<string | null>(null);
  const setError = (value: string | null) => {
    error = value;
  };
  const state = { current: closed },
    wanted = { current: null } as { current: Identity | null },
    generation = { current: 0 },
    revision = { current: -1 },
    opening = { current: false },
    mounted = { current: true };
  const closeFailed = { current: false };
  const openTask = { current: null } as { current: Promise<void> | null };
  const closeQueue = { current: Promise.resolve(true) } as {
    current: Promise<boolean>;
  };
  function matches(value: QuickLookStatus) {
    return (
      wanted.current?.scanId === value.scanId &&
      wanted.current?.nodeId === value.nodeId
    );
  }
  function accept(value: QuickLookStatus, fromCommand = false) {
    if (
      !mounted.current ||
      !value ||
      !Number.isSafeInteger(value.revision) ||
      value.revision <= revision.current
    )
      return;
    revision.current = value.revision;
    if (value.open && !matches(value)) return;
    if (!value.open && (fromCommand || state.current.open || !opening.current))
      wanted.current = null;
    state.current = value;
    setStatus(value);
  }
  function enqueueClose(request: number) {
    const task = closeQueue.current
      .then(async () => {
        // Every newer open awaits this queue; the prior close executes first.
        // Only the current context may receive its status or recovery warning.
        const result = await invoke<QuickLookStatus>("close_quick_look");
        if (
          !result ||
          result.open !== false ||
          !Number.isSafeInteger(result.revision)
        )
          throw new Error("Invalid Quick Look close response");
        if (request === generation.current && !wanted.current) {
          accept(result, true);
          closeFailed.current = false;
          if (mounted.current) setError(null);
        } else {
          revision.current = Math.max(revision.current, result.revision);
        }
        return true;
      })
      .catch((failure) => {
        if (request === generation.current) {
          closeFailed.current = true;
          if (mounted.current) setError(String(failure));
        }
        return false;
      });
    closeQueue.current = task;
    return task;
  }
  function dismiss() {
    if (!wanted.current && !state.current.open && !closeFailed.current) return;
    const request = ++generation.current;
    wanted.current = null;
    state.current = {
      ...state.current,
      open: false,
      scanId: null,
      nodeId: null,
    };
    if (mounted.current) setStatus(state.current);
    void enqueueClose(request);
    // Registry cancellation is immediate, but also close after the pending
    // opener settles; an obsolete response cannot restore the UI or panel.
    if (openTask.current)
      void openTask.current.then(() => {
        if (request === generation.current && !wanted.current)
          void enqueueClose(request);
      });
  }
  function open(identity: Identity) {
    if (!isTauri() || opening.current || !mounted.current) return false;
    const request = ++generation.current;
    const fence = closeQueue.current;
    wanted.current = identity;
    opening.current = true;
    setPending(true);
    setError(null);
    const task = (async () => {
      try {
        const closedPrevious = await fence;
        if (request !== generation.current || !wanted.current) return;
        if (!closedPrevious) {
          closeFailed.current = true;
          throw new Error(
            "Could not close the previous preview. Dismiss it before opening another file.",
          );
        }
        const result = await invoke<QuickLookStatus>(
          "quick_look_node",
          identity,
        );
        if (request === generation.current && wanted.current)
          accept(result, true);
        else if (result && Number.isSafeInteger(result.revision))
          revision.current = Math.max(revision.current, result.revision);
      } catch (failure) {
        if (mounted.current && request === generation.current) {
          wanted.current = null;
          setError(String(failure));
        }
      } finally {
        opening.current = false;
        if (mounted.current) setPending(false);
      }
    })();
    openTask.current = task;
    void task.then(() => {
      if (openTask.current === task) openTask.current = null;
    });
    return true;
  }
  onMount(() => {
    mounted.current = true;
    if (!isTauri())
      return () => {
        mounted.current = false;
      };
    let cancelled = false;
    const registration = listen<QuickLookStatus>(
      "quick-look-status",
      (event) => {
        if (!cancelled) accept(event.payload);
      },
    );
    void registration.catch(() => {
      if (!cancelled) setError("Quick Look status updates are unavailable.");
    });
    return () => {
      cancelled = true;
      dismiss();
      mounted.current = false;
      void registration.then((unlisten) => unlisten()).catch(() => {});
    };
  });
  return {
    get status() {
      return status;
    },
    get pending() {
      return pending;
    },
    get error() {
      return error;
    },
    dismiss,
    open,
    isPending: () => opening.current,
    retryClose: () => {
      closeFailed.current = true;
      dismiss();
    },
  };
}
