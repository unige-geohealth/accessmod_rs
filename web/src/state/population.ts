// Toolbox › Adjust population distribution (AM5 `amPopulationBarrierCorrection`):
// the draft, its update and the job it makes. Known population: each zone
// gets the population of a field of the zone layer; otherwise each zone
// keeps the population of the raster, moved off barriers.
import type { Key } from "@accessmod/ui/i18n";
import type { Jobs } from "../generated/jobs";
import type { Api } from "../generated/types";
import { fileOf, normalizeTags, pickField } from "./catalogue";

export const ANALYSIS = "adjust-population-distribution";

export interface PopulationDraft {
  population: number | null;
  landCover: number | null;
  /** AM5 mode "known": the zones hold the population. */
  known: boolean;
  zones: number | null;
  /** Population field of the zones (known mode); "" until chosen. */
  populationField: string;
  tags: string;
}

export const EMPTY: PopulationDraft = { population: null, landCover: null, known: false, zones: null, populationField: "", tags: "" };

export type PopulationMessage =
  | { type: "population"; id: number | null }
  | { type: "landCover"; id: number | null }
  | { type: "known"; on: boolean }
  | { type: "zones"; id: number | null }
  | { type: "populationField"; field: string }
  | { type: "tags"; text: string };

export function updatePopulation(draft: PopulationDraft, message: PopulationMessage): PopulationDraft {
  switch (message.type) {
    case "population":
      return { ...draft, population: message.id };
    case "landCover":
      return { ...draft, landCover: message.id };
    case "known":
      return { ...draft, known: message.on };
    case "zones":
      // Fields belong to a layer.
      return message.id === draft.zones ? draft : { ...draft, zones: message.id, populationField: "" };
    case "populationField":
      return { ...draft, populationField: message.field };
    case "tags":
      return { ...draft, tags: message.text };
  }
}

/** The draft as shown: unchosen layers take their default (AM5: the first layer of the class). */
export function withPopulationDefaults(
  draft: PopulationDraft,
  first: { population: number | null; landCover: number | null; zones: number | null },
  fieldsOf: (layer: number | null) => readonly string[],
): PopulationDraft {
  const population = draft.population ?? first.population;
  const landCover = draft.landCover ?? first.landCover;
  const zones = draft.zones ?? first.zones;
  const populationField = pickField(draft.populationField, fieldsOf(zones), ["pop", "population"]);
  if (population === draft.population && landCover === draft.landCover && zones === draft.zones && populationField === draft.populationField) {
    return draft;
  }
  return { ...draft, population, landCover, zones, populationField };
}

export interface Context {
  datasets: ReadonlyMap<number, Api.Dataset>;
  dem: number | null;
  fieldsOf: (layer: number | null) => readonly string[];
}

export interface Built {
  document: Omit<Jobs.AdjustPopulationDistributionJob, "output_dir"> | null;
  issues: Key[];
  tags: string[];
}

/** The job of a draft, or what is missing, in AM5 order. */
export function buildJob(draft: PopulationDraft, context: Context): Built {
  const landCover = fileOf(context, draft.landCover);
  const population = fileOf(context, draft.population);
  const zones = fileOf(context, draft.zones);
  const dem = fileOf(context, context.dem);
  const tags = normalizeTags(draft.tags);
  const issues: Key[] = [];
  if (landCover === undefined) issues.push("srv_pop_correction_missing_lcm_layer");
  if (population === undefined) issues.push("srv_pop_correction_missing_population_layer");
  if (zones === undefined) issues.push("srv_pop_correction_missing_zone_layer");
  if (tags.length === 0) issues.push("srv_pop_correction_missing_tags");
  if (draft.known && draft.populationField === "") issues.push("srv_pop_correction_missing_population_column");
  if (issues.length > 0 || landCover === undefined || population === undefined || zones === undefined || dem === undefined) {
    return { document: null, issues, tags };
  }
  // AM5 needs no zone identifier; `zones.csv` takes the usual one.
  const idField = pickField("", context.fieldsOf(draft.zones), ["cat", "id"]);
  return {
    document: {
      schema_version: 1,
      dem,
      population,
      land_cover: landCover,
      zones: { path: zones, id_field: idField, ...(draft.known ? { population_field: draft.populationField } : {}) },
    },
    issues,
    tags,
  };
}
