type DialogOptions = {
  onClose: () => void;
  opener?: HTMLElement | null;
  initialFocus?: string;
};
const focusable =
  'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], summary, [tabindex]:not([tabindex="-1"])';

/** Keep keyboard focus inside a modal, then return it to its opener. */
export function dialogFocus(node: HTMLElement, initial: DialogOptions) {
  let options = initial;
  const opener =
    initial.opener ??
    (document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null);
  let alive = true;
  const controls = () =>
    [...node.querySelectorAll<HTMLElement>(focusable)]
      .filter((element) => !element.closest("[hidden]"))
      // A closed <details> hides its content from focus; only its own summary stays reachable.
      .filter((element) => {
        const closed = element.closest("details:not([open])");
        return !closed || (element.matches("summary") && element.parentElement === closed);
      })
      .sort((a, b) =>
        a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING
          ? -1
          : 1,
      );
  queueMicrotask(() => {
    if (!alive) return;
    const items = controls();
    const preferred = options.initialFocus
      ? node.querySelector<HTMLElement>(options.initialFocus)
      : null;
    (preferred && items.includes(preferred)
      ? preferred
      : (items[0] ?? node)
    ).focus();
  });
  function keydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      options.onClose();
    } else if (event.key === "Tab") {
      const items = controls(),
        first = items[0],
        last = items.at(-1);
      if (!first) {
        event.preventDefault();
        node.focus();
      } else if (
        event.shiftKey &&
        (document.activeElement === first || document.activeElement === node)
      ) {
        event.preventDefault();
        last!.focus();
      } else if (
        !event.shiftKey &&
        (document.activeElement === last || document.activeElement === node)
      ) {
        event.preventDefault();
        first.focus();
      }
    }
  }
  node.addEventListener("keydown", keydown);
  return {
    update(next: DialogOptions) {
      options = next;
    },
    destroy() {
      alive = false;
      node.removeEventListener("keydown", keydown);
      // Svelte removes the modal and clears background inert in the same update.
      // Focusing during action teardown can still hit the inert background.
      queueMicrotask(() => {
        if (opener?.isConnected && document.activeElement === document.body)
          opener.focus();
      });
    },
  };
}

/** Light-dismiss a toolbar popover without stealing focus on outside clicks. */
export function dismissPopover(node: HTMLElement, onClose: () => void) {
  function outside(event: PointerEvent) {
    if (
      event.target instanceof Node &&
      !node.parentElement?.contains(event.target)
    )
      onClose();
  }
  function escape(event: KeyboardEvent) {
    if (event.key === "Escape" && !node.hidden) {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      const triggers = node.parentElement;
      (
        triggers?.querySelector<HTMLButtonElement>(
          '[aria-controls][aria-expanded="true"]',
        ) ?? triggers?.querySelector<HTMLButtonElement>("[aria-controls]")
      )?.focus();
    }
  }
  document.addEventListener("pointerdown", outside);
  node.parentElement?.addEventListener("keydown", escape);
  return {
    destroy() {
      document.removeEventListener("pointerdown", outside);
      node.parentElement?.removeEventListener("keydown", escape);
    },
  };
}

/** Capture activation before a command disables its control or makes it inert. */
export function captureDialogOpener(
  node: HTMLElement,
  remember: (element: HTMLElement) => void,
) {
  function activate(event: MouseEvent) {
    if (
      !(event.target instanceof Element) ||
      event.target.closest('[role="dialog"]')
    )
      return;
    const button = event.target.closest<HTMLButtonElement>("button");
    if (button && node.contains(button)) remember(button);
  }
  node.addEventListener("click", activate, true);
  return {
    destroy() {
      node.removeEventListener("click", activate, true);
    },
  };
}
