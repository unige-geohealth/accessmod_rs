// Declarative list of AM5 states to capture.
//
// Each screen is reached from a fresh navigation to its sidebar tab, then
// `steps` are applied in order. Steps only open views: never click buttons
// that import, delete, rename, archive or run anything.
//
// Step kinds:
//   { tab: "module_data" }                  sidebar item (shinydashboard tabName)
//   { radio: "moduleSelector", value: "module_3" }   AM5 inline button group
//   { panel: "#manageData", index: 2 }       open accordion item (1-based)
//   { check: "#showAdminTools" }              tick a UI-only checkbox
//   { fill: true }                            pick first option of empty visible selects
//   { select: "#id", label: "regex" }         pick the option whose label matches
//   { input: "#id", value: "1351" }           type into a text input
//   { subtab: "i18n_key" }                    Bootstrap nav-tab whose label has that key
//   { click: "selector" }                     help/modal openers, read-only updates

const analysis = (id, value, name, source, { fill = true } = {}) => ({
  id: `analysis-${id}`,
  name: `Analysis › ${name}`,
  source: ["modules/amManageAnalysis/amUi.R", "modules/amAnalysisAccessibility/amUi.R", ...source],
  steps: [{ tab: "module_analysis" }, { radio: "moduleSelector", value }, ...(fill ? [{ fill: true }] : [])],
});

const toolbox = (id, value, name, source, extra = []) => ({
  id: `toolbox-${id}`,
  name: `Toolbox › ${name}`,
  source: ["modules/amManageToolbox/amUi.R", source],
  steps: [{ tab: "module_toolbox" }, { radio: "toolSelector", value }, ...extra, { fill: true }],
});

export const screens = [
  // Project
  {
    id: "project-open",
    name: "Project › Open",
    source: ["modules/amManageProject/amUi.R"],
    steps: [{ tab: "module_project" }, { panel: "#manageProject", index: 1 }, { panel: "#projectSummary", index: 1 }],
  },
  ...[
    [2, "new-import", "New / Import"],
    [3, "delete", "Delete"],
    [4, "export", "Export"],
    [5, "language", "Language"],
  ].map(([index, id, name]) => ({
    id: `project-${id}`,
    name: `Project › ${name}`,
    source: ["modules/amManageProject/amUi.R"],
    steps: [{ tab: "module_project" }, { panel: "#manageProject", index }],
  })),
  {
    id: "project-summary-grid",
    name: "Project › Summary › Grid parameters",
    source: ["modules/amManageProject/amUi.R"],
    steps: [{ tab: "module_project" }, { panel: "#projectSummary", index: 3 }],
  },

  // Data
  {
    id: "data-import",
    name: "Data › Import",
    source: ["modules/amManageData/amUi.R"],
    steps: [{ tab: "module_data" }, { panel: "#manageData", index: 1 }],
  },
  ...[
    [2, "filter", "Filter"],
    [3, "rename", "Rename"],
    [4, "archive", "Archive"],
    [5, "delete", "Delete"],
  ].map(([index, id, name]) => ({
    id: `data-${id}`,
    name: `Data › ${name}`,
    source: ["modules/amManageData/amUi.R"],
    steps: [{ tab: "module_data" }, { panel: "#manageData", index }],
  })),

  // Analysis
  analysis("accessibility", "module_2", "Accessibility", [
    "modules/amAnalysisAccessibility/amUi_modulesData.R",
    "modules/amAnalysisAccessibility/amUi_modulesSettings.R",
    "modules/amAnalysisAccessibility/amUi_tableScenario.R",
    "modules/amAnalysisAccessibility/amUi_tablesFacilities.R",
  ]),
  analysis("coverage", "module_3", "Geographic coverage", [
    "modules/amAnalysisAccessibility/amUi_modulesData.R",
    "modules/amAnalysisAccessibility/amUi_modulesSettings.R",
    "modules/amAnalysisAccessibility/amUi_tablesFacilities.R",
  ]),
  analysis("referral", "module_4", "Referral", [
    "modules/amAnalysisAccessibility/amUi_modulesData.R",
    "modules/amAnalysisAccessibility/amUi_modulesSettings.R",
    "modules/amAnalysisAccessibility/amUi_tablesFacilities.R",
  ]),
  {
    // The table needs a valid max travel time (default 0 is rejected) and
    // the Validation "Update" button; it is a read-only statistic.
    ...analysis("zonal-statistics", "module_5", "Zonal statistics", [
      "modules/amAnalysisAccessibility/amUi_modulesData.R",
      "modules/amAnalysisAccessibility/amUi_zonalStat.R",
      "modules/amAnalysisAccessibility/amUi_modulesValidation.R",
    ], { fill: false }),
    idleTimeout: 10000, // the table output waits for Update
    steps: [
      { tab: "module_analysis" },
      { radio: "moduleSelector", value: "module_5" },
      { select: "#popSelect", label: "^\\(r\\) population \\[demo" },
      { select: "#travelTimeSelect", label: "\\[ca\\]" },
      { select: "#zoneSelect", label: "zone" },
      { fill: true },
      { input: "#textTimeCumCosts", value: "1351" },
      { click: "#btnZonalStat" },
    ],
  },
  analysis("scaling-up", "module_6", "Scaling up", [
    "modules/amAnalysisAccessibility/amUi_modulesData.R",
    "modules/amAnalysisAccessibility/amUi_scalingUpTablesSettings.R",
    "modules/amAnalysisAccessibility/amUi_tablesScalingUp.R",
  ]),
  analysis("optimization", "module_7", "Optimization", [
    "modules/amAnalysisAccessibility/amUi_modulesData.R",
    "modules/amAnalysisAccessibility/amUi_modulesSettings.R",
  ]),

  // Toolbox
  toolbox("map-raster", "tool_map_preview", "Interactive map", "modules/amGisPreview/amUi.R", [
    { panel: "#mapTools", index: 2 },
  ]),
  toolbox("map-options", "tool_map_preview", "Interactive map › Options", "modules/amGisPreview/amUi.R", [
    { panel: "#mapTools", index: 1 },
  ]),
  toolbox("map-facilities", "tool_map_preview", "Interactive map › Facilities", "modules/amGisPreview/amUi.R", [
    { panel: "#mapTools", index: 3 },
  ]),
  toolbox("merge-land-cover", "tool_merge_landcover", "Merge land cover", "modules/amAnalysisMergeLandCover/amUi.R", [
    { subtab: "toolbox_land_cover_tab" },
  ]),
  ...[
    ["roads", "toolbox_land_cover_roads_tab", "Roads"],
    ["barriers", "toolbox_land_cover_barrier_tab", "Barriers"],
    ["stack", "toolbox_land_cover_merge_tab", "Merge (stack)"],
  ].map(([id, subtab, name]) =>
    toolbox(`merge-land-cover-${id}`, "tool_merge_landcover", `Merge land cover › ${name}`, "modules/amAnalysisMergeLandCover/amUi.R", [
      { subtab },
      ...(id === "stack" ? [{ panel: "#stackTable", index: 1 }] : []),
    ]),
  ),
  toolbox("adjust-population", "tool_pop_correction", "Adjust population distribution", "modules/amPopulationBarrierCorrection/amUi.R"),

  // Others
  {
    id: "logs",
    name: "Logs",
    source: ["modules/amManageLogs/amUi.R"],
    steps: [{ tab: "module_logs" }],
  },
  {
    id: "settings",
    name: "Settings",
    source: ["modules/amManageSettings/amUi.R"],
    steps: [{ tab: "module_settings" }],
  },
  {
    id: "settings-admin",
    name: "Settings › Admin tools",
    source: ["modules/amManageSettings/amUi.R"],
    steps: [{ tab: "module_settings" }, { check: "#showAdminTools" }, { check: "#showDevelTools" }],
  },
  {
    id: "about",
    name: "About",
    source: ["modules/amAbout/amUi.R"],
    steps: [{ tab: "module_about" }],
  },
];

// Elements cropped once (en, light) as component references.
// Each entry: [file name, selector, optional screen id to reach first]
export const components = [
  ["header", ".main-header", "project-open"],
  ["sidebar", ".main-sidebar", "project-open"],
  ["sidebar-menu", ".sidebar-menu", "data-import"],
  ["accordion-project", "#manageProject", "project-open"],
  ["accordion-data", "#manageData", "data-import"],
  ["center-title", "#shiny-tab-module_data .col-md-8 h2, #shiny-tab-module_data h2", "data-import"],
  ["data-table", "#shiny-tab-module_data .tabulator", "data-import"],
  ["module-tabs", "#moduleSelector", "analysis-accessibility"],
  ["tool-tabs", "#toolSelector", "toolbox-map-raster"],
  ["form-select", "#shiny-tab-module_analysis .form-group:has(.selectize-control)", "analysis-accessibility"],
  ["scenario-table", "#shiny-tab-module_analysis .tabulator", "analysis-accessibility"],
  ["slider", ".irs", "toolbox-map-raster"],
  ["map-controls", "#shiny-tab-module_toolbox .leaflet-control-container .leaflet-top.leaflet-left", "toolbox-map-raster"],
  ["map", "#shiny-tab-module_toolbox .leaflet-container", "toolbox-map-raster"],
  ["radio-group", "#shiny-tab-module_logs .shiny-input-radiogroup", "logs"],
  ["logs-sidebar", "#shiny-tab-module_logs .well", "logs"],
  ["checkbox", "#shiny-tab-module_settings .checkbox", "settings"],
  ["buttons", "#shiny-tab-module_settings .well", "settings-admin"],
  ["validation", "#shiny-tab-module_analysis .well:has(.fa-square-check)", "analysis-accessibility"],
  ["section-card", "#shiny-tab-module_analysis .well:has(.fa-right-to-bracket)", "analysis-accessibility"],
  ["link-actions", "#shiny-tab-module_analysis .link-button", "analysis-accessibility"],
  // callout.png was cropped from the empty zonal state (no selection)
  ["zonal-table", "#zoneCoverageTable", "analysis-zonal-statistics"],
];

export const languages = ["en", "fr", "es", "pt"];
