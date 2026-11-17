import { expect, it } from "vitest";
import { createQueueMarker, queuePathKey } from "./queueMarkers";

it("marks exact paths, descendants and parents without marking prefix siblings", () => {
  const mark = createQueueMarker([
    { path: "/Downloads/games/" },
    { path: "/Downloads/movie.mp4" },
  ]);
  expect(mark({ path: "/Downloads/games" })).toBe("queued");
  expect(mark({ path: "/Downloads/games/clip.mp4" })).toBe("queued");
  expect(mark({ path: "/Downloads" })).toBe("contains");
  expect(mark({ path: "/" })).toBe("contains");
  expect(mark({ path: "/Downloads/movie.mp4" })).toBe("queued");
  expect(mark({ path: "/Downloads/games-old" })).toBeNull();
  expect(mark({ path: "/Download" })).toBeNull();
  expect(mark({ path: "Other files" })).toBeNull();
});

it("handles root coverage and ignores relative aggregate labels", () => {
  const mark = createQueueMarker([{ path: "/" }, { path: "Other files" }]);
  expect(mark({ path: "/" })).toBe("queued");
  expect(mark({ path: "/Downloads/a" })).toBe("queued");
  expect(mark({ path: "Other files" })).toBeNull();
  expect(
    createQueueMarker([{ path: "Other files" }])({ path: "/Other files" }),
  ).toBeNull();
});

it("uses fresh membership when a new queue revision is supplied", () => {
  const oldMark = createQueueMarker([{ path: "/Downloads/a" }]);
  expect(oldMark({ path: "/Downloads/a" })).toBe("queued");
  const newMark = createQueueMarker([]);
  expect(newMark({ path: "/Downloads/a" })).toBeNull();
  expect(newMark({ path: "/Downloads" })).toBeNull();
});

it("uses native byte keys for collisions and their parent/child markers", () => {
  const first = { path: "/scope/native-�", pathKey: "/scope/native-%FF" };
  const second = { path: first.path, pathKey: "/scope/native-%FE" };
  const mark = createQueueMarker([first]);
  expect(mark(first)).toBe("queued");
  expect(mark(second)).toBeNull();
  expect(mark({ path: "/scope" })).toBe("contains");
  expect(
    mark({ path: first.path + "/child", pathKey: first.pathKey + "/child" }),
  ).toBe("queued");
  expect(
    mark({ path: second.path + "/child", pathKey: second.pathKey + "/child" }),
  ).toBeNull();
  expect(mark({ path: "/scope/native-%FF" })).toBeNull();
  expect(mark({ path: first.path })).toBeNull();
  expect(queuePathKey({ path: "/scope/é %/'()!~*_.-" })).toBe(
    "/scope/%C3%A9%20%25/'()!~*_.-",
  );
});
