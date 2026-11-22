import { describe, expect, it } from "vitest";
import { byteSize } from "./size";

describe("byteSize", () => {
  it("shows bytes below a kilobyte", () => {
    expect(byteSize(0)).toBe("0 B");
    expect(byteSize(999)).toBe("999 B");
  });
  it("uses decimal units with one decimal", () => {
    expect(byteSize(1000)).toBe("1.0 kB");
    expect(byteSize(1_500_000)).toBe("1.5 MB");
    expect(byteSize(8_800_000_000)).toBe("8.8 GB");
    expect(byteSize(2e15)).toBe("2000.0 TB");
  });
  it("never shows nonsense", () => {
    expect(byteSize(Number.NaN)).toBe("0 B");
    expect(byteSize(-5)).toBe("0 B");
  });
});
