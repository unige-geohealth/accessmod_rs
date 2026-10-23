// User-visible text. English only for now; keys and vocabulary follow the
// AccessMod 5 dictionary (www/dictionary/main.json, classes.json), which an
// xtask will convert into catalogues for en, fr, es and pt.

const en = {
  "app.title": "AccessMod",
  "section.projects": "Project",
  "section.data": "Data",
  "section.analysis": "Analysis",
  "section.toolbox": "Toolbox",
  "section.logs": "Logs",
  "section.settings": "Settings",
  "section.about": "About",
  "analysis.travel-time": "Accessibility",
  "analysis.geographic-coverage": "Geographic coverage",
  "analysis.referral": "Referral",
  "analysis.zonal-statistics": "Zonal statistics",
  "analysis.scaling-up": "Scaling up",
  "analysis.optimization": "Optimization",
  "analysis.merge-land-cover": "Merge land cover",
  "analysis.adjust-population-distribution": "Adjust population distribution",
  "project.new": "New project",
  "project.name": "Name",
  "project.description": "Description",
  "project.dem": "DEM (file on this computer)",
  "project.create": "Create",
  "project.open": "Open",
  "project.none": "No project yet: create one from a DEM.",
  "data.import": "Import a dataset",
  "data.file": "File on this computer",
  "data.class": "Class",
  "data.tags": "Tags",
  "data.layer": "Layer (vector files)",
  "data.empty": "No dataset.",
  "data.remove": "Remove",
  "data.preview": "Preview",
  "job.saved": "Saved jobs",
  "job.new": "New",
  "job.name": "Job name",
  "job.save": "Save",
  "job.validate": "Validate",
  "job.run": "Run",
  "job.tags": "Output tags",
  "job.undo": "Undo",
  "job.redo": "Redo",
  "job.valid": "Valid",
  "job.queued": "Run queued: follow it in Logs.",
  "logs.empty": "No run yet.",
  "logs.cancel": "Cancel",
  "map.basemap": "Online basemap",
  "map.value": "Value",
  "common.loading": "Loading…",
  "common.error": "Error",
} as const;

export type Key = keyof typeof en;

export function t(key: Key): string {
  return en[key];
}

/** Display name of an analysis. */
export function analysisName(name: string): string {
  const key = `analysis.${name}` as Key;
  return key in en ? en[key] : name;
}

/** Analysis screens and toolbox tools, in AccessMod 5 order. */
export const ANALYSIS_SCREENS = [
  "travel-time",
  "geographic-coverage",
  "referral",
  "zonal-statistics",
  "scaling-up",
  "optimization",
] as const;
export const TOOLBOX_SCREENS = ["merge-land-cover", "adjust-population-distribution"] as const;
