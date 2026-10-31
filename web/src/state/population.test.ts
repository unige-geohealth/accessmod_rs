import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { buildJob, EMPTY, updatePopulation, withPopulationDefaults } from "./population";

const dataset = (id: number, file: string) => ({ id, file }) as Api.Dataset;
const datasets = new Map([1, 2, 3, 4].map((id) => [id, dataset(id, `datasets/${id}.x`)]));
const fieldsOf = (layer: number | null) => (layer === 4 ? ["name", "cat", "pop_known"] : []);
const context = { datasets, dem: 1, fieldsOf };

describe("adjust population draft", () => {
  it("lists what is missing, in AM5 order", () => {
    expect(buildJob({ ...EMPTY, known: true }, context).issues).toEqual([
      "srv_pop_correction_missing_lcm_layer",
      "srv_pop_correction_missing_population_layer",
      "srv_pop_correction_missing_zone_layer",
      "srv_pop_correction_missing_tags",
      "srv_pop_correction_missing_population_column",
    ]);
  });

  it("takes the first layers and builds the job, population known or not", () => {
    const draft = withPopulationDefaults({ ...EMPTY, tags: "adjusted" }, { population: 2, landCover: 3, zones: 4 }, fieldsOf);
    expect(draft.populationField).toBe("name");
    const unknown = buildJob(draft, context);
    expect(unknown.document).toEqual({
      schema_version: 1,
      dem: "datasets/1.x",
      population: "datasets/2.x",
      land_cover: "datasets/3.x",
      zones: { path: "datasets/4.x", id_field: "cat" },
    });
    const known = updatePopulation(updatePopulation(draft, { type: "known", on: true }), { type: "populationField", field: "pop_known" });
    expect(buildJob(known, context).document?.zones).toEqual({ path: "datasets/4.x", id_field: "cat", population_field: "pop_known" });
    expect(buildJob(known, context).tags).toEqual(["adjusted"]);
  });

  it("forgets the field of another zone layer", () => {
    const draft = updatePopulation({ ...EMPTY, zones: 4, populationField: "pop_known" }, { type: "zones", id: 5 });
    expect(draft.populationField).toBe("");
  });
});
