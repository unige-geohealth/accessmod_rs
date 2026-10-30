import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { classLabel, datasetOptions, fieldClasses, filterFromQuery, pairedTable, filterToQuery, kindOf, matches, normalizeTags, toRow } from "./catalogue";

const dataset = (id: number, cls: Api.DataClass, tags: string[]) => ({ id, class: cls, tags }) as Api.Dataset;
const t = (key: string) => key;

describe("catalogue", () => {
  it("normalises tags as the server does", () => {
    expect(normalizeTags("travel_time Santé  santé")).toEqual(["travel", "time", "Sante", "sante"]);
    expect(normalizeTags("Straße œuvre")).toEqual(["Strasse", "oeuvre"]);
    expect(normalizeTags(" -- ")).toEqual([]);
    // Alphabetic symbols and other numerics, as Rust's is_alphanumeric.
    expect(normalizeTags("ⓐ½ x")).toEqual(["ⓐ½", "x"]);
  });

  it("knows class kinds and labels", () => {
    expect(kindOf("vFacility")).toBe("vector");
    expect(kindOf("rDem")).toBe("raster");
    expect(classLabel((k) => k, "rLandCover")).toBe("land cover");
    expect(classLabel((k) => (k === "class.rLandCover" ? "occupation du sol" : k), "rLandCover")).toBe("occupation du sol");
  });

  it("filters by type, text and tags", () => {
    const rows = [dataset(1, "rDem", ["demo"]), dataset(2, "vFacility", ["health", "demo"])].map((d) => toRow(d, t as never));
    const ids = (q: Record<string, string>) => rows.filter((r) => matches(r, filterFromQuery(q), new Set([2]))).map((r) => r.id);
    expect(ids({})).toEqual([1, 2]);
    expect(ids({ type: "vector" })).toEqual([2]);
    expect(ids({ type: "bogus" })).toEqual([1, 2]);
    expect(ids({ text: "health" })).toEqual([2]);
    expect(ids({ text: "Health" })).toEqual([]);
    expect(ids({ tags: "demo, health" })).toEqual([2]);
    expect(ids({ last: "1" })).toEqual([2]);
    expect(ids({ last: "1", type: "raster" })).toEqual([]);
  });

  it("round-trips the filter through the URL query", () => {
    const filter = { type: "raster" as const, text: "a", tags: "", last: true };
    expect(filterFromQuery(filterToQuery(filter))).toEqual(filter);
    expect(filterToQuery({ type: "all", text: "", tags: "", last: false })).toEqual({ type: "", text: "", tags: "", last: "" });
  });

  it("offers the datasets of a job field's classes, and pairs tables by tags", () => {
    const schema = { properties: { land_cover: { "x-accessmod-class": ["rLandCoverMerged"] } } };
    expect(fieldClasses(schema, "land_cover")).toEqual(["rLandCoverMerged"]);
    expect(fieldClasses(schema, "nope")).toEqual([]);
    const list = [dataset(1, "rLandCoverMerged", ["a"]), dataset(2, "rDem", ["a"]), dataset(3, "rLandCoverMerged", ["b", "c"])];
    expect(datasetOptions(t, list, ["rLandCoverMerged"])).toEqual([
      { value: "3", label: "(r) land cover merged [b c]" },
      { value: "1", label: "(r) land cover merged [a]" },
    ]);
    const tables = [dataset(4, "tLandCover", ["c", "b"]), dataset(5, "tLandCover", ["b"])];
    expect(pairedTable(tables, list[2], "tLandCover")?.id).toBe(4);
    expect(pairedTable(tables, list[0], "tLandCover")).toBeUndefined();
  });
});
