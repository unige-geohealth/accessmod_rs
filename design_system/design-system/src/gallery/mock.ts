// Static data copied from the AM5 DEMO project (references/screens/*/dom.html).
import { dataLabel, type Option } from "@/components/am/form";
import type { Row } from "@/components/am/data-table";

const opt = (type: "r" | "v" | "t", cls: string, tags: string): Option => ({
  value: `${type}:${cls}:${tags}`,
  label: dataLabel(type, cls, tags),
});

export const layers = {
  population: [opt("r", "population", "demo patients")],
  populationResidual: [opt("r", "population residual", "demo patients")],
  landCoverMerged: [opt("r", "land cover merged", "demo")],
  scenario: [opt("t", "scenario processed", "bicycling only"), opt("t", "scenario", "demo")],
  facilities: [opt("v", "facility", "demo")],
  travelTime: [opt("r", "travel time", "bicycling only")],
  capacity: [opt("t", "capacity processed", "to remove")],
  exclusion: [opt("t", "exclusion processed", "to remove")],
  dataClasses: [
    { value: "rDem", label: "(r) dem" },
    { value: "rLandCover", label: "(r) land cover" },
    { value: "vFacility", label: "(v) facility" },
  ],
};

export const fields = {
  id: [{ value: "cat", label: "cat" }],
  name: [{ value: "name", label: "name" }],
  capacity: [{ value: "capacity", label: "capacity" }],
};

export const scenarioToProcess: Row[] = [1, 2, 3, 1001, 1002, 1003].map((c) => ({
  class: c,
  label: `no_label_${c}`,
  speed: 0,
  mode: "WALKING",
}));

export const scenarioSelected: Row[] = [
  { class: 1, label: "Built_area", speed: 12, mode: "BICYCLING" },
  { class: 2, label: "Low_dense_vegetation", speed: 3, mode: "BICYCLING" },
  { class: 3, label: "Dense_vegetation", speed: 1, mode: "BICYCLING" },
  { class: 1001, label: "Main road", speed: 20, mode: "BICYCLING" },
  { class: 1002, label: "Secondary", speed: 20, mode: "BICYCLING" },
  { class: 1003, label: "Tertiary", speed: 20, mode: "BICYCLING" },
];

export const facilities: Row[] = [
  ["Chabvala", 3, 233.3], ["Queen Elizabeth", 1003, 3129.87], ["Neno Rural", 3, 360.85], ["Chiradzulu", 1001, 805.2],
  ["Namadzi", 1002, 551.04], ["Mwanza", 1003, 727.96], ["Chipini Rural", 1003, 230.49], ["Tulokhondo", 3, 89.55],
  ["Mdeka", 1003, 240.65], ["Nkula Clinic", 3, 64.71],
].map(([name, lc, pop], i) => ({
  cat: i + 1,
  amOnBarrier: "no",
  amOnZero: "yes",
  amCatLandCover: lc as number,
  amPopCell: pop as number,
  name: name as string,
}));

export const dataList: Row[] = [
  ["to remove", "table", "exclusion processed"],
  ["demo", "table", "land cover"],
  ["demo", "table", "scenario"],
  ["bicycling only", "table", "scenario processed"],
  ["scenario velo", "table", "scenario processed"],
  ["test normal", "table", "scenario processed"],
  ["referral", "table", "referral"],
  ["single keepnet replay", "table", "referral"],
  ["referral", "table", "referral nearest by dist"],
  ["referral", "table", "referral nearest by time"],
  ["demo", "raster", "dem"],
  ["demo patients", "raster", "population"],
  ["demo", "vector", "facility"],
  ["demo", "vector", "road"],
].map(([tags, type, cls]) => ({ tags, type, class: cls }));

export const logs: Row[] = [
  ["2026-10-30 08:19", "log", "Analysis accessibility: travel time computed in 12 s"],
  ["2026-10-30 08:18", "message", "Project demo opened"],
  ["2026-10-30 08:12", "warning", "Some labels in the selected scenario table do not match"],
  ["2026-10-30 08:02", "error", "Facilities located outside DEM"],
].map(([time, type, msg]) => ({ time, type, msg }));

export const zonalLayers = {
  travelTime: [opt("r", "travel time", "ca"), opt("r", "travel time", "bicycling only")],
  zones: [opt("v", "zone for stat", "demo")],
  zoneId: [{ value: "cat", label: "cat" }],
  zoneName: [{ value: "admin_name", label: "admin_name" }],
};

export const zonalTable: Row[] = [
  [1, "South West", 1634957.715299514, 1634957.715299514, 100],
  [2, "North East", 1325294.544360217, 1321071.030634936, 99.68131509004893],
  [3, "South East", 1146542.042503255, 1146542.042503255, 100],
  [4, "North West", 33635.1243987084, 33635.1243987084, 100],
].map(([cat, admin_name, popTotal, tt, pct]) => ({
  cat,
  admin_name,
  popTotal,
  popTravelTime_1351: tt,
  popCoveredPercent_1351: pct,
})) as Row[];

export const landCover = {
  rasters: [opt("r", "land cover", "demo")],
  tables: [opt("t", "land cover", "demo")],
  rasterLabels: [1, 2, 3].map((c) => ({ class: c, label: "" })) as Row[],
  tableLabels: [
    { class: 1, label: "Built_area" },
    { class: 2, label: "Low_dense_vegetation" },
    { class: 3, label: "Dense_vegetation" },
  ] as Row[],
  roads: [opt("v", "road", "demo")],
  roadClass: [{ value: "class", label: "class" }],
  roadLabel: [{ value: "label", label: "label" }],
  barriers: [opt("v", "barrier", "demo river"), opt("v", "barrier", "demo wetland")],
};

export const stack = {
  use: [] as { id: string; label: string; kind: "raster" | "vector" }[],
  skip: [
    ["stack barrier [demo river line]", "vector"],
    ["stack barrier [demo wetland area]", "raster"],
    ["stack barrier [demo wetland line]", "vector"],
    ["stack land cover [demo grid]", "raster"],
    ["stack road [Main road 64 fixed line]", "vector"],
    ["stack road [Main road demo line]", "vector"],
    ["stack road [Secondary 64 fixed line]", "vector"],
    ["stack road [Secondary demo line]", "vector"],
    ["stack road [Tertiary 64 fixed line]", "vector"],
    ["stack road [Tertiary demo line]", "vector"],
  ].map(([label, kind]) => ({ id: label, label, kind: kind as "raster" | "vector" })),
};

/** Facility points around Blantyre, Malawi (demo project area). */
export const facilityPoints: [number, number][] = [
  [34.83, -15.43], [34.93, -15.45], [35.02, -15.38], [35.12, -15.5], [35.08, -15.62],
  [34.79, -15.6], [34.9, -15.72], [35.0, -15.7], [35.15, -15.66], [34.85, -15.52],
];
