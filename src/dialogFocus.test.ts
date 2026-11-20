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
