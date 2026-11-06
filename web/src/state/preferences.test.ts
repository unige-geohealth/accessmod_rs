import { afterEach, describe, expect, it, vi } from "vitest";
import { readPreference, setPreference } from "./preferences";

describe("preferences", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("keeps a choice in memory when storage refuses it", () => {
    const refusing = {
      getItem: () => null,
      setItem: () => {
        throw new Error("denied");
      },
      removeItem: () => {
        throw new Error("denied");
      },
    };
    vi.stubGlobal("localStorage", refusing);
    expect(readPreference("overwrite")).toBe(false);
    setPreference("overwrite", true);
    expect(readPreference("overwrite")).toBe(true);
    setPreference("overwrite", false);
    expect(readPreference("overwrite")).toBe(false);
  });

  it("reads and writes storage", () => {
    const store = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => store.set(k, v),
      removeItem: (k: string) => store.delete(k),
    });
    setPreference("overwrite", true);
    expect(store.get("am.overwrite")).toBe("1");
    expect(readPreference("overwrite")).toBe(true);
    setPreference("overwrite", false);
    expect(store.has("am.overwrite")).toBe(false);
  });
});
