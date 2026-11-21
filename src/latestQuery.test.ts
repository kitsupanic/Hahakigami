import { expect, it, vi } from "vitest";
import { latestOnly } from "./latestQuery";

it("runs the queued replacement after a running job rejects", async () => {
  const error = vi.spyOn(console, "error").mockImplementation(() => {});
  const run = latestOnly();
  const replacement = vi.fn(async () => {});
  run(() => Promise.reject(new Error("boom")));
  run(replacement);
  await vi.waitFor(() => expect(replacement).toHaveBeenCalledOnce());
  expect(error).toHaveBeenCalledOnce();
  error.mockRestore();
});
