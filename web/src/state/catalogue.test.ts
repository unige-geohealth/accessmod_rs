import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { classLabel, plannedOutputs, tagSuggestions, datasetIds, datasetOptions, fieldClasses, filterFromQuery, jobRow, pairedTable, filterToQuery, kindOf, matches, normalizeTags, runRoles, toRow } from "./catalogue";

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

  it("names a job as a dataset: the analysis as class, the tags naming it", () => {
    const run = (job_tags: string[]) => ({ id: 9, analysis: "travel-time", tags: ["walking"], job_tags }) as unknown as Api.RunRecord;
    const row = jobRow(run(["walking", "9"]), (k) => k);
    expect([row.type, row.class, row.tags]).toEqual(["data.kind_job", "analysis_accessibility_main_tab", "walking 9"]);
    expect(jobRow(run([]), (k) => k).tags).toBe("walking");
    expect(jobRow(run([]), () => "Fusionner l'occupation du sol").class).toBe("fusionner l'occupation du sol");
  });

  it("filters by type, text, tags and run", () => {
    const run = { id: 7, analysis: "travel-time", tags: ["demo"], inputs: [1], datasets: [2] } as Api.RunRecord;
    const rows = [...[dataset(1, "rDem", ["demo"]), dataset(2, "vFacility", ["health", "demo"]), dataset(3, "rDem", ["other"])].map((d) => toRow(d, t as never)), jobRow(run, t as never)];
    const roles = runRoles(run);
    const keys = (q: Record<string, string>) => rows.filter((r) => matches(r, filterFromQuery(q), roles)).map((r) => r.key);
    expect(keys({ run: "7" })).toEqual(["d1", "d2", "j7"]);
    expect(keys({ run: "7", type: "job" })).toEqual(["j7"]);
    expect(keys({ type: "config" })).toHaveLength(4);
    expect([...roles]).toEqual([["j7", "job"], ["d1", "input"], ["d2", "output"]]);
    expect(datasetIds(["d1", "j7", "d2"])).toEqual([1, 2]);
    const ids = (q: Record<string, string>) => rows.slice(0, 2).filter((r) => matches(r, filterFromQuery(q))).map((r) => r.id);
    expect(ids({})).toEqual([1, 2]);
    expect(ids({ type: "vector" })).toEqual([2]);
    expect(ids({ type: "bogus" })).toEqual([1, 2]);
    expect(ids({ text: "health" })).toEqual([2]);
    expect(ids({ text: "Health" })).toEqual([]);
    expect(ids({ tags: "demo, health" })).toEqual([2]);
  });

  it("plans the outputs of a run and the datasets they would replace", () => {
    const list = [dataset(1, "rTravelTime", ["a", "b"]), dataset(2, "rTravelTime", ["a"]), dataset(3, "rDem", ["a", "b"])];
    expect(plannedOutputs("travel-time", ["a", "b"], list).map((o) => [o.class, o.existing?.id])).toEqual([
      ["rTravelTime", 1],
      ["rNearest", undefined],
    ]);
    expect(plannedOutputs("travel-time", [], list)).toEqual([]);
    expect(plannedOutputs("nope", ["a"], list)).toEqual([]);
  });

  it("marks outputs a queued or running job already writes", () => {
    const job = (id: number, status: Api.RunStatus, analysis: string, tags: string[]) =>
      ({ id, status, analysis, tags, kind: "analysis", job: {}, output_dir: "", started: 0, datasets: [], inputs: [] }) as Api.RunRecord;
    const runs = [job(4, "queued", "travel-time", ["a"]), job(5, "done", "travel-time", ["b"]), job(6, "running", "travel-time", ["b"])];
    const reserved = (tags: string[]) => plannedOutputs("travel-time", tags, [], runs).map((o) => o.reservedBy);
    expect(reserved(["a"])).toEqual([4, 4]);
    expect(reserved(["b"])).toEqual([6, 6]);
    expect(reserved(["c"])).toEqual([undefined, undefined]);
    // Another analysis writing other classes does not reserve them.
    expect(plannedOutputs("travel-time", ["z"], [], [job(7, "queued", "zonal-statistics", ["z"])]).map((o) => o.reservedBy)).toEqual([undefined, undefined]);
  });

  it("suggests the tag sets of the analysis, newest first, and the next free number", () => {
    const at = (id: number, cls: Api.DataClass, tags: string[]) => ({ ...dataset(id, cls, tags), created: id }) as Api.Dataset;
    const list = [at(1, "rTravelTime", ["test", "3"]), at(2, "rTravelTime", ["test", "4"]), at(3, "rTravelTime", ["other"]), at(4, "tZonalStat", ["test", "9"])];
    const queued = [{ id: 7, status: "queued", analysis: "travel-time", kind: "analysis", tags: ["test", "5"], started: 10 }] as Api.RunRecord[];
    const show = (text: string, runs: Api.RunRecord[] = []) =>
      tagSuggestions("travel-time", text, list, runs).map((s) => `${s.tags.join(" ")} (${s.note.kind === "job" ? `job ${s.note.run}` : s.note.kind})`);
    expect(show("")).toEqual(["other (exists)", "test 4 (exists)", "test 3 (exists)"]);
    // Words typed start tags; zonal statistics outputs are not travel-time ones.
    expect(show("te")).toEqual(["test 4 (exists)", "test 3 (exists)"]);
    expect(show("test")).toEqual(["test 5 (free)", "test 4 (exists)", "test 3 (exists)"]);
    // A queued job holds its set: the free one skips it and it comes first.
    expect(show("test 4", queued)).toEqual(["test 6 (free)", "test 4 (exists)"]);
    expect(show("test", queued)).toEqual(["test 6 (free)", "test 5 (job 7)", "test 4 (exists)", "test 3 (exists)"]);
    // A taken set without a number: the free one is numbered 2; a free text gets none.
    expect(show("Other")[0]).toBe("Other 2 (free)");
    expect(show("new")).toEqual([]);
    expect(show("test 5")).toEqual([]);
    // A number beyond exact arithmetic is not a counter (no endless search).
    const huge = [at(9, "rTravelTime", ["big", "9007199254740992"])];
    expect(tagSuggestions("travel-time", "big", huge).map((x) => x.note.kind)).toEqual(["exists"]);
  });

  it("round-trips the filter through the URL query", () => {
    const filter = { type: "raster" as const, text: "a", tags: "", run: 3 };
    expect(filterFromQuery(filterToQuery(filter))).toEqual(filter);
    expect(filterToQuery({ type: "all", text: "", tags: "", run: null })).toEqual({ type: "", text: "", tags: "", run: "" });
    expect(filterFromQuery({ run: "x" }).run).toBeNull();
    expect(filterFromQuery({ run: "-1" }).run).toBeNull();
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
    // Class by class, as AM5 lists them: the population before newer residuals.
    const population = [dataset(6, "rPopulation", ["a"]), dataset(7, "rPopulationResidual", ["r"]), dataset(8, "rPopulation", ["b"])];
    expect(datasetOptions(t, population, ["rPopulation", "rPopulationResidual"]).map((o) => o.value)).toEqual(["8", "6", "7"]);
    const tables = [dataset(4, "tLandCover", ["c", "b"]), dataset(5, "tLandCover", ["b"])];
    expect(pairedTable(tables, list[2], "tLandCover")?.id).toBe(4);
    expect(pairedTable(tables, list[0], "tLandCover")).toBeUndefined();
  });
});
