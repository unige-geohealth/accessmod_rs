import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { classLabel, filterFromQuery, filterToQuery, kindOf, matches, normalizeTags, toRow } from "./catalogue";

const dataset = (id: number, cls: Api.DataClass, tags: string[]) => ({ id, class: cls, tags }) as Api.Dataset;
const label = (code: Api.DataClass) => code;

describe("catalogue", () => {
  it("normalises tags as the server does", () => {
    expect(normalizeTags("travel_time Santé  santé")).toEqual(["travel", "time", "Sante", "sante"]);
    expect(normalizeTags("Straße œuvre")).toEqual(["Strasse", "oeuvre"]);
    expect(normalizeTags(" -- ")).toEqual([]);
  });

  it("knows class kinds and labels", () => {
    expect(kindOf("vFacility")).toBe("vector");
    expect(kindOf("rDem")).toBe("raster");
    expect(classLabel((k) => k, "rLandCover")).toBe("land cover");
    expect(classLabel((k) => (k === "class.rLandCover" ? "occupation du sol" : k), "rLandCover")).toBe("occupation du sol");
  });

  it("filters by type, text and tags", () => {
    const rows = [dataset(1, "rDem", ["demo"]), dataset(2, "vFacility", ["health", "demo"])].map((d) => toRow(d, label));
    const ids = (q: Record<string, string>) => rows.filter((r) => matches(r, filterFromQuery(q))).map((r) => r.id);
    expect(ids({})).toEqual([1, 2]);
    expect(ids({ type: "vector" })).toEqual([2]);
    expect(ids({ type: "bogus" })).toEqual([1, 2]);
    expect(ids({ text: "health" })).toEqual([2]);
    expect(ids({ text: "Health" })).toEqual([]);
    expect(ids({ tags: "demo, health" })).toEqual([2]);
  });

  it("round-trips the filter through the URL query", () => {
    const filter = { type: "raster" as const, text: "a", tags: "" };
    expect(filterFromQuery(filterToQuery(filter))).toEqual(filter);
    expect(filterToQuery({ type: "all", text: "", tags: "" }).type).toBe("");
  });
});
