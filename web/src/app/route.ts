// View state in the URL hash: `#/<project>/<section>/<panel>?<query>`, `_`
// when no project is open (slugs never contain `_`). Reloading or sharing a
// link shows the same screen, panel and filters.
import { useSyncExternalStore } from "react";

/** AM5 sections, in AM5 order; Jobs takes the place of AM5 Logs. */
export const SECTIONS = ["project", "data", "analysis", "toolbox", "jobs", "settings", "about"] as const;
export type Section = (typeof SECTIONS)[number];

export interface Route {
  project: string | null;
  section: Section;
  /** Open accordion panel of the left column; `null` when all are closed. */
  panel: string | null;
  /** Screen view state (filters), as strings. */
  query: Readonly<Record<string, string>>;
}

export const HOME: Route = { project: null, section: "project", panel: null, query: {} };

const NO_PROJECT = "_";

const isSection = (s: string): s is Section => (SECTIONS as readonly string[]).includes(s);

/** A segment of the hash, decoded; a malformed escape gives `null`. */
function decode(segment: string): string | null {
  try {
    return decodeURIComponent(segment);
  } catch {
    return null;
  }
}

export function parseRoute(hash: string): Route {
  const raw = hash.replace(/^#\/?/, "");
  const at = raw.indexOf("?");
  const path = at < 0 ? raw : raw.slice(0, at);
  const search = at < 0 ? "" : raw.slice(at + 1);
  const [p = NO_PROJECT, s = "", panel = ""] = path.split("/").map((x) => decode(x) ?? "");
  const query: Record<string, string> = {};
  for (const [k, v] of new URLSearchParams(search)) query[k] = v;
  return {
    project: p === NO_PROJECT || p === "" ? null : p,
    section: isSection(s) ? s : "project",
    panel: panel === "" ? null : panel,
    query,
  };
}

export function formatRoute(route: Route): string {
  const parts = [route.project ?? NO_PROJECT, route.section];
  if (route.panel !== null) parts.push(route.panel);
  const path = parts.map(encodeURIComponent).join("/");
  const entries = Object.entries(route.query).filter(([, v]) => v !== "");
  const search = entries.length > 0 ? `?${new URLSearchParams(entries).toString()}` : "";
  return `#/${path}${search}`;
}

function subscribe(listener: () => void) {
  window.addEventListener("hashchange", listener);
  return () => window.removeEventListener("hashchange", listener);
}

let cached: { hash: string; route: Route } | null = null;

/** The current route; a new object only when the hash changes. */
function current(): Route {
  const hash = window.location.hash;
  if (cached?.hash !== hash) cached = { hash, route: parseRoute(hash) };
  return cached.route;
}

export function useRoute(): Route {
  return useSyncExternalStore(subscribe, current);
}

export function navigate(route: Route) {
  window.location.hash = formatRoute(route);
}

/** Go to `section` of the current project, panels closed and filters reset. */
export function goSection(route: Route, section: Section) {
  navigate({ ...route, section, panel: null, query: {} });
}
