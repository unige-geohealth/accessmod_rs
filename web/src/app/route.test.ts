import { describe, expect, it } from "vitest";
import { formatRoute, HOME, parseRoute } from "./route";

describe("route", () => {
  it("round-trips project, section, panel and query", () => {
    const route = { project: "demo", section: "data" as const, panel: "filter", query: { type: "raster", text: "a b" } };
    expect(parseRoute(formatRoute(route))).toEqual(route);
  });

  it("uses _ when no project is open", () => {
    expect(formatRoute(HOME)).toBe("#/_/project");
    expect(parseRoute("#/_/data")).toEqual({ ...HOME, section: "data" });
  });

  it("falls back on bad input", () => {
    expect(parseRoute("")).toEqual(HOME);
    expect(parseRoute("#/demo/nowhere")).toEqual({ ...HOME, project: "demo" });
    expect(parseRoute("#/%E0%A4%A/data").project).toBeNull();
  });

  it("drops empty query values", () => {
    expect(formatRoute({ ...HOME, query: { text: "" } })).toBe("#/_/project");
  });
});
