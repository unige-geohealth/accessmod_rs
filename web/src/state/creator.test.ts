import { describe, expect, it } from "vitest";
import { type CreatorDraft, EMPTY, missing, params, update } from "./creator";

const apply = (...messages: Parameters<typeof update>[1][]): CreatorDraft => messages.reduce(update, EMPTY);

describe("creator draft", () => {
  it("toggles units and starts afresh with another country or level", () => {
    let d = apply({ type: "country", iso3: "SSD" }, { type: "toggleUnit", id: "a" }, { type: "toggleUnit", id: "b" }, { type: "toggleUnit", id: "a" });
    expect(d.units).toEqual(["b"]);
    expect(update(d, { type: "country", iso3: "SSD" })).toBe(d);
    d = update(d, { type: "level", level: 2 });
    expect(d.units).toEqual([]);
    d = update(update(d, { type: "toggleUnit", id: "c" }), {
      type: "country",
      iso3: "RWA",
    });
    expect(d.units).toEqual([]);
  });

  it("says what is missing, then gives the command parameters", () => {
    expect(missing(EMPTY)).toEqual(["creator.need_name", "creator.need_country"]);
    const named = apply({ type: "name", text: "Juba county" }, { type: "country", iso3: "SSD" });
    expect(missing(named)).toEqual(["creator.need_units"]);
    expect(params(named)).toBeNull();
    const chosen = update(update(named, { type: "toggleUnit", id: "u1" }), {
      type: "buffer",
      km: 5,
    });
    expect(params(chosen)).toEqual({
      name: "Juba county",
      iso3: "SSD",
      level: 1,
      units: ["u1"],
      buffer_m: 5000,
      resolution_m: 100,
      layers: EMPTY.layers,
      default_scenarios: true,
    });
    // The whole country needs no unit.
    const country = update(named, { type: "level", level: 0 });
    expect(params(country)?.units).toEqual([]);
  });

  it("switches layers off one by one", () => {
    const d = apply({ type: "layer", layer: "osm_facilities", on: false });
    expect(d.layers.osm_facilities).toBe(false);
    expect(d.layers.roads).toBe(true);
  });

  it("declines the default scenarios", () => {
    expect(EMPTY.default_scenarios).toBe(true);
    const d = apply({ type: "defaultScenarios", on: false });
    expect(d.default_scenarios).toBe(false);
    const named = update(update(update(d, { type: "name", text: "Juba" }), { type: "country", iso3: "SSD" }), { type: "level", level: 0 });
    expect(params(named)?.default_scenarios).toBe(false);
  });
});
