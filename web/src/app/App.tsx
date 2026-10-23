import { useEvents } from "../api/events";
import { useProject } from "../api/queries";
import { About } from "../views/About";
import { AnalysisScreen } from "../views/Analysis";
import { DataScreen } from "../views/Data";
import { LogsScreen } from "../views/Logs";
import { ProjectsScreen } from "../views/Projects";
import { SettingsScreen } from "../views/Settings";
import { analysisName, t, useLanguage } from "./i18n";
import { navigate, type Route, type Section, useRoute } from "./route";

const PROJECT_SECTIONS: Section[] = ["data", "analysis", "toolbox", "logs"];

export function App() {
  const route = useRoute();
  // Every text follows the language: a change renders the whole tree again.
  const language = useLanguage();
  useEvents(route.project);
  return (
    <div className="shell" key={language} lang={language}>
      <nav className="sidebar">
        <div className="brand">{t("app.title")}</div>
        <NavLink route={route} target={{ project: null, section: "projects", item: null }} />
        {route.project !== null && (
          <>
            <ProjectName slug={route.project} />
            {PROJECT_SECTIONS.map((section) => (
              <NavLink
                key={section}
                route={route}
                target={{ project: route.project, section, item: null }}
              />
            ))}
          </>
        )}
        <div className="spacer" />
        <NavLink
          route={route}
          target={{ project: route.project, section: "settings", item: null }}
        />
        <NavLink route={route} target={{ project: route.project, section: "about", item: null }} />
      </nav>
      <main className="content">
        <Screen route={route} />
      </main>
    </div>
  );
}

function NavLink({ route, target }: { route: Route; target: Route }) {
  const active = route.section === target.section;
  return (
    <button
      type="button"
      className={active ? "nav active" : "nav"}
      onClick={() => navigate(target)}
    >
      {t(`section.${target.section}`)}
    </button>
  );
}

function ProjectName({ slug }: { slug: string }) {
  const project = useProject(slug);
  return <div className="project-name">{project.data?.name ?? slug}</div>;
}

function Screen({ route }: { route: Route }) {
  const project = route.project;
  if (project === null) {
    if (route.section === "about") return <About />;
    if (route.section === "settings") return <SettingsScreen />;
    return <ProjectsScreen />;
  }
  switch (route.section) {
    case "projects":
      return <ProjectsScreen />;
    case "data":
      return <DataScreen project={project} selected={route.item} />;
    case "analysis":
    case "toolbox":
      return (
        <AnalysisScreen
          key={`${route.section}/${route.item ?? ""}`}
          project={project}
          section={route.section}
          analysis={route.item}
          title={route.item ? analysisName(route.item) : null}
        />
      );
    case "logs":
      return <LogsScreen project={project} />;
    case "settings":
      return <SettingsScreen />;
    case "about":
      return <About />;
  }
}
