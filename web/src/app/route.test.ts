import { describe, expect, it } from "vitest";
import { formatRoute, HOME, parseRoute } from "./route";

describe("route", () => {
  it("round-trips", () => {
    const route = { project: "demo niger", section: "analysis" as const, item: "travel-time" };
    expect(parseRoute(formatRoute(route))).toEqual(route);
  });
  it("defaults", () => {
    expect(parseRoute("")).toEqual(HOME);
    expect(parseRoute("#/demo")).toEqual({ project: "demo", section: "data", item: null });
    expect(parseRoute("#/demo/unknown")).toEqual({ project: "demo", section: "data", item: null });
  });
});
