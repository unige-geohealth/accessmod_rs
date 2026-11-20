// The AM5 frame: sidebar sections, current project, version; the screen of
// the current route.
import { AppShell, type NavItem, sections } from "@accessmod/ui/am/app-shell";
import { Callout } from "@accessmod/ui/am/messages";
import who from "@accessmod/ui/assets/who.svg";
import { useI18n } from "@accessmod/ui/i18n";
import { useEvents } from "../api/events";
import { useHealth, useProject, useQueue } from "../api/queries";
import { AnalysisScreen } from "../screens/analysis";
import { DataScreen } from "../screens/data";
import { JobsScreen } from "../screens/jobs";
import { AboutScreen } from "../screens/other";
import { ProjectScreen } from "../screens/project";
import { SettingsScreen } from "../screens/settings";
import { ToolboxScreen } from "../screens/toolbox";
import { useFmt } from "../screens/common";
import { JobPanel } from "./job-panel";
import { Notifications } from "./notifications";
import { goSection, type Route, SECTIONS, type Section, useRoute } from "./route";

const isSection = (s: string): s is Section => (SECTIONS as readonly string[]).includes(s);

/** The AM5 sidebar, Jobs in place of Logs. */
const NAV: NavItem[] = sections.map((s) => (s.id === "logs" ? { id: "jobs", label: "jobs.nav", icon: "logs" } : s));

export function App() {
  const { t } = useI18n();
  const f = useFmt();
  const route = useRoute();
  // One stream for the whole app, every project the user may see: a
  // change of route never reconnects it, and a job of any project is
  // notified (docs/ui.md, "Notifications").
  useEvents(null);
  const health = useHealth();
  const project = useProject(route.project);
  // Jobs queued or running, in every lane: the badge of Jobs.
  const active = useQueue().data?.length ?? 0;
  const nav = NAV.map((s) => (s.id === "jobs" ? { ...s, badge: active, badgeLabel: f("jobs.active_count", active) } : s));
  return (
    <AppShell
      product="AccessMod 6"
      logo={<img src={who} alt={t("app.who_logo")} className="max-h-10 w-full px-[15px]" />}
      project={project.data?.name ?? route.project ?? "—"}
      version={health.data ? `${t("menu_version_current")} ${health.data.version}` : ""}
      active={route.section}
      sections={nav}
      onNavigate={(s) => {
        if (isSection(s)) goSection(route, s);
      }}
    >
      <Screen route={route} />
      <Notifications />
      <JobPanel />
    </AppShell>
  );
}

function Screen({ route }: { route: Route }) {
  switch (route.section) {
    case "project":
      return <ProjectScreen route={route} />;
    case "data":
      return route.project === null ? <Callout k="project.open_first" /> : <DataScreen key={route.project} route={route} project={route.project} />;
    case "toolbox":
      return <ToolboxScreen route={route} />;
    case "analysis":
      return route.project === null ? <Callout k="project.open_first" /> : <AnalysisScreen key={route.project} route={route} project={route.project} />;
    case "jobs":
      return <JobsScreen route={route} />;
    case "settings":
      return <SettingsScreen />;
    case "about":
      return <AboutScreen />;
  }
}
