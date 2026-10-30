import type { ReactNode } from "react";
import { AnalysisScreen } from "./screens/analysis";
import { Catalogue } from "./screens/catalogue";
import { AboutScreen, DataScreen, LogsScreen, ProjectScreen, SettingsScreen } from "./screens/sections";
import { ToolboxScreen } from "./screens/toolbox";

export type Route = {
  /** Gallery route, also the AM5 reference id (references/screens/<id>). */
  id: string;
  /** Sidebar section highlighted in the shell. */
  section: string;
  render: () => ReactNode;
  /** No AM5 reference (design-system only page). */
  noReference?: boolean;
};

export const routes: Route[] = [
  { id: "catalogue", section: "", render: () => <Catalogue />, noReference: true },
  { id: "project-open", section: "project", render: () => <ProjectScreen /> },
  { id: "data-import", section: "data", render: () => <DataScreen /> },
  { id: "analysis-accessibility", section: "analysis", render: () => <AnalysisScreen initial="accessibility" /> },
  { id: "analysis-zonal-statistics", section: "analysis", render: () => <AnalysisScreen initial="zonal" /> },
  { id: "analysis-scaling-up", section: "analysis", render: () => <AnalysisScreen initial="scalingUp" /> },
  { id: "toolbox-map-raster", section: "toolbox", render: () => <ToolboxScreen initial="map" /> },
  { id: "toolbox-merge-land-cover", section: "toolbox", render: () => <ToolboxScreen initial="merge" /> },
  { id: "toolbox-merge-land-cover-roads", section: "toolbox", render: () => <ToolboxScreen initial="merge" mergeTab="roads" /> },
  { id: "toolbox-merge-land-cover-barriers", section: "toolbox", render: () => <ToolboxScreen initial="merge" mergeTab="barriers" /> },
  { id: "toolbox-merge-land-cover-stack", section: "toolbox", render: () => <ToolboxScreen initial="merge" mergeTab="merge" /> },
  { id: "toolbox-adjust-population", section: "toolbox", render: () => <ToolboxScreen initial="pop" /> },
  { id: "logs", section: "logs", render: () => <LogsScreen /> },
  { id: "settings-admin", section: "settings", render: () => <SettingsScreen /> },
  { id: "about", section: "about", render: () => <AboutScreen /> },
];

/** Default screen for each sidebar section when navigating from the shell. */
export const sectionRoute: Record<string, string> = {
  project: "project-open",
  data: "data-import",
  analysis: "analysis-accessibility",
  toolbox: "toolbox-map-raster",
  logs: "logs",
  settings: "settings-admin",
  about: "about",
};
