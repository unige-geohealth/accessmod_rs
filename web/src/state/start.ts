// The project the client opens when the URL names none: the server's default
// project if it still exists, else the demo project. Pure selectors.
import type { Api } from "../generated/types";

type Entry = Pick<Api.ProjectEntry, "slug"> & { info: Pick<Api.ProjectInfo, "demo"> };

export const demoSlug = (projects: readonly Entry[]): string | null => projects.find((p) => p.info.demo === true)?.slug ?? null;

export function startProject(projects: readonly Entry[], settings: Api.Settings): string | null {
  const preferred = settings.default_project;
  if (preferred && projects.some((p) => p.slug === preferred)) return preferred;
  return demoSlug(projects);
}
