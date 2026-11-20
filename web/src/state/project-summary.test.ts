import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { extentDegrees, extentMetric, gridParameters, nameTaken, projectOptions, validName } from "./project-summary";

const grid: Api.GridInfo = { cols: 227, rows: 196, geo_transform: [600000, 1000, 0, 8500000, 0, -1000], crs_wkt: "" };

describe("project summary", () => {
  it("lists the AM5 grid parameters", () => {
    expect(Object.fromEntries(gridParameters(grid))).toEqual({
      nsres: "1000",
      ewres: "1000",
      rows: "196",
      cols: "227",
      cells: "44492",
    });
  });

  it("gives extents in metres and degrees", () => {
    expect(Object.fromEntries(extentMetric(grid))).toEqual({ north: "8500000", south: "8304000", east: "827000", west: "600000" });
    expect(extentDegrees([33.9, -16.6, 35.9, -14.2])[0]).toEqual(["north", "-14.200000"]);
  });

  it("needs four characters for a name", () => {
    expect(validName(" ab ")).toBe(false);
    expect(validName("demo")).toBe(true);
  });
});

describe("nameTaken", () => {
  const projects = [{ slug: "valais-2", info: { name: "Valais 2" } }];
  it("compares trimmed, case-folded names with projects and projects being made", () => {
    expect(nameTaken(" valais 2 ", projects, [])).toBe(true);
    expect(nameTaken("Valais 3", projects, [])).toBe(false);
    expect(nameTaken("Ticino", projects, [{ name: "ticino", operation: "project-create", finished: null }])).toBe(true);
    // Done, or a removal: the name is free.
    expect(nameTaken("Ticino", projects, [{ name: "Ticino", operation: "project-create", finished: 5 }])).toBe(false);
    expect(nameTaken("Ticino", projects, [{ name: "Ticino", operation: "project-remove", finished: null }])).toBe(false);
    expect(nameTaken("", projects, undefined)).toBe(false);
    // Renamed: its own name is free.
    expect(nameTaken("valais 2", projects, [], "valais-2")).toBe(false);
  });
});

describe("projectOptions", () => {
  it("adds the slug only to names that collide", () => {
    expect(
      projectOptions([
        { slug: "valais-2", info: { name: "valais 2" } },
        { slug: "valais-2-2", info: { name: "Valais 2 " } },
        { slug: "demo", info: { name: "Demo" } },
      ]).map((o) => o.label),
    ).toEqual(["valais 2 (valais-2)", "Valais 2 (valais-2-2)", "Demo"]);
  });
});
