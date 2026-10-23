import { describe, expect, it } from "vitest";
import scalingUp from "../../schemas/jobs/scaling-up.json";
import travelTime from "../../schemas/jobs/travel-time.json";
import {
  datasetClasses,
  defaultValue,
  enumOptions,
  fieldKind,
  getIn,
  nullable,
  properties,
  removeIn,
  type Schema,
  setIn,
  variantOf,
  variants,
} from "./schema";

const tt = travelTime as Schema;
const su = scalingUp as Schema;
const prop = (root: Schema, name: string): Schema => {
  const found = properties(root, root).find(([n]) => n === name);
  if (!found) throw new Error(name);
  return found[1];
};

describe("schema model", () => {
  it("reads kinds, enums, optional fields and dataset classes", () => {
    expect(fieldKind(tt, prop(tt, "dem"))).toBe("string");
    expect(datasetClasses(tt, prop(tt, "dem"))).toEqual(["rDem"]);
    expect(fieldKind(tt, prop(tt, "method"))).toBe("enum");
    expect(enumOptions(tt, prop(tt, "method"))).toContain("anisotropic");
    expect(enumOptions(tt, prop(tt, "neighbourhood"))).toEqual([8, 16]);
    expect(fieldKind(tt, prop(tt, "limit_s"))).toBe("nullable");
    expect(nullable(tt, prop(tt, "limit_s"))?.type).toBe("number");
    expect(fieldKind(tt, prop(tt, "scenario"))).toBe("array");
    expect(datasetClasses(tt, prop(tt, "facilities"))).toEqual(["vFacility", "vFacilityNew"]);
  });

  it("reads tagged, external and constant variants", () => {
    const exclusion = properties(su, prop(su, "exclusions")).length;
    expect(exclusion).toBe(0);
    const suitability = prop(su, "suitability");
    const factor = (suitability as { items: Schema }).items;
    const factorVariants = variants(su, factor);
    expect(factorVariants?.map((v) => v.label)).toEqual([
      "population",
      "distance",
      "travel_time",
      "priority",
    ]);
    const priority = factorVariants?.[3];
    const layer =
      priority?.kind === "tagged"
        ? properties(su, priority.schema).find(([n]) => n === "layer")
        : undefined;
    expect(layer && datasetClasses(su, layer[1])).toEqual(["rPriority"]);
    const layerVariants = layer ? variants(su, layer[1]) : null;
    // A priority factor takes a raster only.
    expect(layerVariants?.map((v) => `${v.kind}:${v.label}`)).toEqual(["external:raster"]);
    if (layerVariants) {
      expect(variantOf(layerVariants, { raster: "datasets/2.tif" })).toBe(0);
      expect(variantOf(layerVariants, "population")).toBe(-1);
    }
  });

  it("builds defaults for required fields", () => {
    const job = defaultValue(tt, tt) as Record<string, unknown>;
    expect(job.neighbourhood).toBe(8);
    expect(job.schema_version).toBe(0);
    expect(job.method).toBeDefined();
    expect(job.facilities).toEqual({ id_field: "", path: "" });
    expect("limit_s" in job).toBe(false);
  });

  it("edits immutably", () => {
    const doc = { a: [{ b: 1 }, { b: 2 }] };
    const next = setIn(doc, ["a", 1, "b"], 5);
    expect(getIn(next, ["a", 1, "b"])).toBe(5);
    expect(doc.a[1]?.b).toBe(2);
    expect(removeIn(next, ["a", 0])).toEqual({ a: [{ b: 5 }] });
    expect(setIn({ x: 1, y: 2 }, ["x"], undefined)).toEqual({ y: 2 });
  });
});

describe("layer fields", () => {
  const factorVariants = () => {
    const factor = (prop(su, "suitability") as { items: Schema }).items;
    return variants(su, factor) ?? [];
  };
  const layerOf = (label: string) => {
    const v = factorVariants().find((f) => f.label === label);
    if (v?.kind !== "tagged") throw new Error(label);
    const layer = properties(su, v.schema).find(([n]) => n === "layer");
    if (!layer) throw new Error("layer");
    return layer[1];
  };
  it("offer only the layers each factor takes", () => {
    const labels = (label: string) => variants(su, layerOf(label))?.map((v) => v.label);
    expect(labels("priority")).toEqual(["raster"]);
    expect(labels("population")).toEqual(["population", "raster"]);
    expect(labels("distance")).toEqual(["facilities", "vector"]);
  });
  it("default to an accepted layer", () => {
    expect(defaultValue(su, layerOf("population"))).toBe("population");
    expect(defaultValue(su, layerOf("priority"))).toEqual({ raster: "" });
  });
});
