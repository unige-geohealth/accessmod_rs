// View state in the URL hash: `#/<project>/<section>/<item>`. Reloading or
// sharing a link shows the same screen.
import { useSyncExternalStore } from "react";

export const SECTIONS = [
  "projects",
  "data",
  "analysis",
  "toolbox",
  "logs",
  "settings",
  "about",
] as const;
export type Section = (typeof SECTIONS)[number];

export interface Route {
  project: string | null;
  section: Section;
  item: string | null;
}

export const HOME: Route = { project: null, section: "projects", item: null };

/** A path segment, or `null` when it is not valid percent-encoding. */
function decode(part: string): string | null {
  try {
    return decodeURIComponent(part);
  } catch {
    return null;
  }
}

export function parseRoute(hash: string): Route {
  const parts = hash
    .replace(/^#\/?/, "")
    .split("/")
    .filter((p) => p.length > 0)
    .map(decode);
  if (parts.some((p) => p === null)) return HOME;
  const [project, section, item] = parts as string[];
  if (project === undefined) return HOME;
  const known = SECTIONS.find((s) => s === section);
  return { project, section: known ?? "data", item: item ?? null };
}

export function formatRoute(route: Route): string {
  if (route.project === null) return "#/";
  const parts = [route.project, route.section, route.item].filter((p): p is string => p !== null);
  return `#/${parts.map(encodeURIComponent).join("/")}`;
}

function subscribe(listener: () => void) {
  window.addEventListener("hashchange", listener);
  return () => window.removeEventListener("hashchange", listener);
}

export function useRoute(): Route {
  const hash = useSyncExternalStore(subscribe, () => window.location.hash);
  return parseRoute(hash);
}

export function navigate(route: Route) {
  window.location.hash = formatRoute(route);
}
