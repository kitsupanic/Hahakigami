// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import CleanupResults from "./CleanupResults.svelte";
import { act, createTestRoot, type Root } from "./testUtils";
import type { CleanupReport } from "./cleanupTypes";
import type { NodeView } from "./types";

const item = (id: number, path: string): NodeView => ({
  id,
  name: "backup.zip",
  path,
  parentId: 0,
  kind: "file",
  childrenCount: 0,
  logicalBytes: 4096,
  allocatedBytes: 4096,
  incomplete: false,
});
const moved = item(1, "/Downloads/backup.zip");
const failed = item(2, "/External/backup.zip");
const base: CleanupReport = {
  id: 1,
  source: "queue",
  state: "confirmed",
  reviewedCount: 2,
  movedCount: 1,
  failedCount: 1,
  movedItems: [moved],
  failedItems: [{ item: failed, message: "File is in use" }],
  failures: ["File is in use"],
  detailsAvailable: true,
  message: null,
  persistenceWarning: null,
  queueState: "confirmed",
  remainingQueued: 3,
  queueWarning: null,
  queueError: null,
};
let container: HTMLDivElement, root: Root;
const onQueue = vi.fn(),
  onRetry = vi.fn(),
  onDismiss = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement("div");
  document.body.append(container);
  root = createTestRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});
async function render(
  report: Partial<CleanupReport> = {},
  props: { queueOpen?: boolean; disabled?: boolean } = {},
) {
  await act(() =>
    root.render(CleanupResults, {
      report: { ...base, ...report },
      onQueue,
      onRetry,
      onDismiss,
      ...props,
    }),
  );
}
const text = () => container.textContent?.replace(/\s+/g, " ");

it("separates moved, failed and all-location queue counts with exact item paths", async () => {
  await render();
  expect(
    [...container.querySelectorAll("dd")].map((node) => node.textContent),
  ).toEqual(["1", "1", "3"]);
  expect(container.querySelector('[role="status"]')?.textContent).toBe(
    "Moved 1 of 2 items to Trash",
  );
  expect(
    container.querySelector('[aria-label="Items moved to Trash"]')?.textContent,
  ).toContain(moved.path);
  expect(
    container.querySelector('[aria-label="Items that failed to move to Trash"]')
      ?.textContent,
  ).toContain(failed.path);
  expect(
    container.querySelector('[aria-label="Items that failed to move to Trash"]')
      ?.textContent,
  ).toContain("File is in use");
  expect(text()).toContain("Still queued, all locations");
});
it("keeps confirmed moves separate from failed queue bookkeeping", async () => {
  await render({
    persistenceWarning: "Queue could not be saved",
    remainingQueued: 4,
  });
  expect(text()).toContain("Moved 1 of 2 items to Trash");
  expect(text()).toContain("Moved items may still show in it");
  expect(
    [...container.querySelectorAll("dd")].map((node) => node.textContent),
  ).toEqual(["1", "1", "4"]);
});
it("does not claim zero moves when the native result is unconfirmed", async () => {
  await render({
    state: "unconfirmed",
    movedCount: null,
    failedCount: null,
    movedItems: [],
    failedItems: [],
    detailsAvailable: false,
    message: "Connection lost",
  });
  expect(container.querySelector('[role="status"]')?.textContent).toBe(
    "Cleanup result unconfirmed",
  );
  expect(text()).not.toContain("No items moved");
  expect(text()).toContain("Check Trash in Finder");
  expect(text()).toContain("Connection lost");
  expect(
    [...container.querySelectorAll("dd")]
      .slice(0, 2)
      .map((node) => node.textContent),
  ).toEqual(["Unconfirmed", "Unconfirmed"]);
});
it("offers a queue-status retry without offering to retry Trash", async () => {
  await render({
    queueState: "unconfirmed",
    remainingQueued: null,
    queueError: "Queue unavailable",
  });
  const retry = [...container.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "Retry queue status",
  )!;
  await act(() => retry.click());
  expect(onRetry).toHaveBeenCalledOnce();
  expect(
    [...container.querySelectorAll("button")].map((button) =>
      button.textContent?.trim(),
    ),
  ).not.toContain("Move to Trash");
  expect(text()).toContain("how many items are left in the queue");
});
it("distinguishes checking the queue from an empty confirmed queue", async () => {
  await render({ queueState: "checking", remainingQueued: null });
  expect(container.querySelectorAll("dd")[2].textContent).toBe("Checking…");
  await render({ remainingQueued: 0 });
  expect(container.querySelectorAll("dd")[2].textContent).toBe("0");
});
it("drops the count table for a clean cleanup and keeps the queue count in one line", async () => {
  await render({ failedCount: 0, failedItems: [], failures: [] });
  expect(container.querySelector(".cleanup-result-summary")).toBeNull();
  expect(
    container
      .querySelector(".cleanup-result-source")
      ?.textContent?.replace(/\s+/g, " ")
      .trim(),
  ).toBe("From the cleanup queue · 3 items still queued across all locations");
  await render({
    failedCount: 0,
    failedItems: [],
    failures: [],
    remainingQueued: 0,
  });
  expect(
    container.querySelector(".cleanup-result-source")?.textContent?.trim(),
  ).toBe("From the cleanup queue");
  await render({
    failedCount: 0,
    failedItems: [],
    failures: [],
    queueState: "checking",
    remainingQueued: null,
  });
  expect(text()).toContain("Checking the queue…");
  await render({
    failedCount: 0,
    failedItems: [],
    failures: [],
    queueState: "unconfirmed",
    remainingQueued: null,
  });
  expect(container.querySelector(".cleanup-result-summary")).not.toBeNull();
});
it("does not fabricate item results when only aggregate counts were confirmed", async () => {
  await render({ detailsAvailable: false, movedItems: [], failedItems: [] });
  expect(container.querySelectorAll(".cleanup-result-items")).toHaveLength(0);
  expect(text()).toContain("Can't show each item for this cleanup");
  expect(text()).toContain("File is in use");
});
it("preserves opened details during queue refresh and resets them for a new cleanup", async () => {
  await render();
  const details = container.querySelector<HTMLDetailsElement>("details")!;
  details.open = true;
  await render({ remainingQueued: 2 });
  expect(container.querySelector<HTMLDetailsElement>("details")!.open).toBe(
    true,
  );
  await render({ id: 2 });
  expect(container.querySelector<HTMLDetailsElement>("details")!.open).toBe(
    false,
  );
});
it("keeps navigation and dismissal separate from cleanup effects", async () => {
  await render();
  await act(() =>
    [...container.querySelectorAll("button")]
      .find((button) => button.textContent?.trim() === "Review queue")!
      .click(),
  );
  await act(() =>
    [...container.querySelectorAll("button")]
      .find((button) => button.textContent?.trim() === "Done")!
      .click(),
  );
  expect(onQueue).toHaveBeenCalledOnce();
  expect(onDismiss).toHaveBeenCalledOnce();
  await render({}, { queueOpen: true });
  expect(
    [...container.querySelectorAll("button")].some(
      (button) => button.textContent?.trim() === "Review queue",
    ),
  ).toBe(false);
});

const status = () => container.querySelector('[role="status"]')?.textContent;
it.each([
  [{ movedCount: 1, failedCount: 0, failedItems: [] }, "Moved 1 item to Trash"],
  [
    {
      movedCount: 2,
      failedCount: 0,
      failedItems: [],
      movedItems: [moved, moved],
    },
    "Moved 2 items to Trash",
  ],
  [{ movedCount: 10, failedCount: 2 }, "Moved 10 of 12 items to Trash"],
  [
    { movedCount: 0, failedCount: 2, movedItems: [] },
    "No items moved to Trash",
  ],
])("words the headline for %j", async (report, headline) => {
  await render(report);
  expect(status()).toBe(headline);
});
it("shows the recorded size moved with the next step, and none when nothing moved", async () => {
  await render({ movedItems: [moved, { ...moved, allocatedBytes: 8192 }] });
  expect(container.querySelector(".cleanup-result-size")?.textContent).toMatch(
    /12 KiB\s+on disk/,
  );
  expect(text()).toContain("Empty the Trash in Finder to free the space.");
  expect(
    container.querySelector(".cleanup-result-caveat")?.textContent,
  ).toMatch(
    /You may get back less space\. Cloned files, hard links and compression can cause this\./,
  );
  expect(text()).not.toContain("does not immediately free");
  await render({ movedCount: 0, movedItems: [] });
  expect(container.querySelector(".cleanup-result-size")).toBeNull();
  expect(container.querySelector(".cleanup-result-caveat")).toBeNull();
  expect(text()).not.toContain("Empty the Trash");
});
