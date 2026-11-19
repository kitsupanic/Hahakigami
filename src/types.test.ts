import { describe, expect, it } from "vitest";
import { bytes } from "./types";

describe("bytes", () => {
  it("shows binary and decimal sizes for the observed folder allocation", () => {
    expect(bytes(181_033_136_128)).toBe("168.6 GiB (181.03 GB)");
  });

  it("keeps byte-sized values compact", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(512)).toBe("512 B");
  });

  it("uses the appropriate unit independently for each scale", () => {
    expect(bytes(1_000_000)).toBe("976.6 KiB (1.00 MB)");
    expect(bytes(1024 ** 2)).toBe("1 MiB (1.05 MB)");
  });

  it("moves up a unit when rounding reaches the next threshold", () => {
    expect(bytes(1_048_550)).toBe("1 MiB (1.05 MB)");
    expect(bytes(999_999)).toBe("976.6 KiB (1.00 MB)");
    expect(bytes(1_048_550, 1)).toBe("1.0 MiB (1.05 MB)");
  });

  it("preserves requested binary decimal precision", () => {
    expect(bytes(1024 ** 3, 1)).toBe("1.0 GiB (1.07 GB)");
  });
});
