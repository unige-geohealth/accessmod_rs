// Project › New from a region (Project Creator): the draft shared by the
// form (left) and the map (right). Pure update, selectors for the command
// parameters and what is missing (docs/ui.md, "State").
import type { Key } from "@accessmod/ui/i18n";
import type { Api } from "../generated/types";
import { validName } from "./project-summary";
import { createStore } from "./store";

export interface Layers {
  land_cover: boolean;
  population: boolean;
  roads: boolean;
  waterways: boolean;
  osm_facilities: boolean;
}

export interface CreatorDraft {
  name: string;
  iso3: string | null;
  /** geoBoundaries level: 0 is the whole country. */
  level: number;
  /** Units chosen on the map (geoBoundaries `shapeID`). */
  units: string[];
  buffer_km: number;
  resolution_m: number;
  layers: Layers;
  /** Save the travel-time jobs of the preset scenarios (walking, bicycle, motorized). */
  default_scenarios: boolean;
}

export type CreatorMessage =
  | { type: "name"; text: string }
  | { type: "country"; iso3: string | null }
  | { type: "level"; level: number }
  | { type: "toggleUnit"; id: string }
  | { type: "clearUnits" }
  | { type: "buffer"; km: number }
  | { type: "resolution"; metres: number }
  | { type: "layer"; layer: keyof Layers; on: boolean }
  | { type: "defaultScenarios"; on: boolean };

export const BUFFERS_KM = [0, 5, 10, 25, 50] as const;
export const RESOLUTIONS_M = [50, 100, 200, 500, 1000] as const;
export const LEVELS = [0, 1, 2, 3] as const;

export const EMPTY: CreatorDraft = {
  name: "",
  iso3: null,
  level: 1,
  units: [],
  buffer_km: 10,
  resolution_m: 100,
  layers: {
    land_cover: true,
    population: true,
    roads: true,
    waterways: true,
    osm_facilities: true,
  },
  default_scenarios: true,
};

export function update(state: CreatorDraft, message: CreatorMessage): CreatorDraft {
  switch (message.type) {
    case "name":
      return { ...state, name: message.text };
    case "country":
      // Units belong to a country: another country starts afresh.
      return message.iso3 === state.iso3 ? state : { ...state, iso3: message.iso3, units: [] };
    case "level":
      return message.level === state.level ? state : { ...state, level: message.level, units: [] };
    case "toggleUnit":
      return {
        ...state,
        units: state.units.includes(message.id) ? state.units.filter((u) => u !== message.id) : [...state.units, message.id],
      };
    case "clearUnits":
      return state.units.length === 0 ? state : { ...state, units: [] };
    case "buffer":
      return { ...state, buffer_km: message.km };
    case "resolution":
      return { ...state, resolution_m: message.metres };
    case "layer":
      return {
        ...state,
        layers: { ...state.layers, [message.layer]: message.on },
      };
    case "defaultScenarios":
      return { ...state, default_scenarios: message.on };
  }
}

/** What prevents creating the project, in the order to fix it. */
export function missing(d: CreatorDraft): Key[] {
  const out: Key[] = [];
  if (!validName(d.name)) out.push("creator.need_name");
  if (d.iso3 === null) out.push("creator.need_country");
  else if (d.level > 0 && d.units.length === 0) out.push("creator.need_units");
  return out;
}

/** The `creator.create` parameters, or `null` while something is missing. */
export function params(d: CreatorDraft): Api.CreateFromRegion | null {
  if (missing(d).length > 0 || d.iso3 === null) return null;
  return {
    name: d.name.trim(),
    iso3: d.iso3,
    level: d.level,
    units: d.level === 0 ? [] : d.units,
    buffer_m: d.buffer_km * 1000,
    resolution_m: d.resolution_m,
    layers: d.layers,
    default_scenarios: d.default_scenarios,
  };
}

export const creatorStore = createStore(EMPTY, update);
