// Analysis (AM5 `amAnalysisAccessibility`): one tab per analysis, data
// input, settings and validation on the left, the tables of the analysis on
// the right. The draft of each tab is a store per project (state/drafts.ts),
// so it survives navigation; datasets, rows, validation and runs are server
// state.
import { type SectionTab, SectionTabs } from "@accessmod/ui/am/section-tabs";
import { navigate, type Route } from "../app/route";
import { Accessibility } from "./analysis/accessibility";
import { Coverage } from "./analysis/coverage";
import { Optimization } from "./analysis/optimization";
import { Referral } from "./analysis/referral";
import { ScalingUp } from "./analysis/scaling-up";
import { NotYet } from "./other";

type Tab = "accessibility" | "coverage" | "referral" | "zonal" | "scalingUp" | "optimization";

const TABS: SectionTab<Tab>[] = [
  { value: "accessibility", label: "analysis_accessibility_main_tab", icon: "accessibility" },
  { value: "coverage", label: "analysis_coverage_main_tab", icon: "coverage" },
  { value: "referral", label: "analysis_referral_main_tab", icon: "referral" },
  { value: "zonal", label: "analysis_zonal_stats_main_tab", icon: "zonal" },
  { value: "scalingUp", label: "analysis_scalingup_main_tab", icon: "scalingUp" },
  { value: "optimization", label: "analysis_optimization_main_tab", icon: "optimization" },
];

const isTab = (v: string | null): v is Tab => TABS.some((t) => t.value === v);

export function AnalysisScreen({ route, project }: { route: Route; project: string }) {
  const tab = isTab(route.panel) ? route.panel : "accessibility";
  return (
    <div className="flex flex-col gap-5">
      <SectionTabs tabs={TABS} value={tab} onChange={(v) => navigate({ ...route, panel: v, query: {} })} />
      {tab === "accessibility" ? (
        <Accessibility project={project} />
      ) : tab === "coverage" ? (
        <Coverage project={project} />
      ) : tab === "referral" ? (
        <Referral project={project} />
      ) : tab === "scalingUp" ? (
        <ScalingUp project={project} />
      ) : tab === "optimization" ? (
        <Optimization project={project} />
      ) : (
        <NotYet />
      )}
    </div>
  );
}
