// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { dialogFocus } from "./dialogFocus";

afterEach(() => {
  document.body.innerHTML = "";
});

function tab(shiftKey = false) {
  document.activeElement!.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Tab", shiftKey, bubbles: true }),
  );
}

it("cycles Tab over a closed details' summary and skips its hidden content", async () => {
  document.body.innerHTML = `<div id="dialog" tabindex="-1">
    <button id="first">First</button>
    <details><summary id="summary">More</summary><button id="hidden">Hidden</button></details>
  </div>`;
  const dialog = document.getElementById("dialog")!;
  dialogFocus(dialog, { onClose() {} });
  await Promise.resolve();
  expect(document.activeElement?.id).toBe("first");
  const summary = document.getElementById("summary")!;
  summary.focus();
  // The summary is the last stop, so Tab wraps instead of escaping into the hidden button.
  tab();
  expect(document.activeElement?.id).toBe("first");
  tab(true);
  expect(document.activeElement?.id).toBe("summary");
  // Open, the content joins the cycle.
  dialog.querySelector("details")!.open = true;
  document.getElementById("hidden")!.focus();
  tab();
  expect(document.activeElement?.id).toBe("first");
});

it("returns focus to the opener, or to the fallback control once the opener is gone", async () => {
  document.body.innerHTML = `<button id="fallback" data-dialog-fallback>Rescan</button>
    <button id="opener">Row</button><div id="dialog" tabindex="-1"><button>Done</button></div>`;
  const dialog = document.getElementById("dialog")!;
  const opener = document.getElementById("opener")!;
  const close = async (removeOpener: boolean) => {
    const action = dialogFocus(dialog, { onClose() {}, opener });
    await Promise.resolve();
    if (removeOpener) opener.remove();
    (document.activeElement as HTMLElement).blur();
    action.destroy();
    await Promise.resolve();
    return document.activeElement?.id;
  };
  expect(await close(false)).toBe("opener");
  expect(await close(true)).toBe("fallback");
});
