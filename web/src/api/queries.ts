// Server state: cached queries, refreshed by server events, never edited
// locally (docs/ui.md, "State").
import { QueryClient, useQuery } from "@tanstack/react-query";
import { api } from "./client";

export const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 30_000, retry: 1, refetchOnWindowFocus: false } },
});

export const keys = {
  analyses: ["analyses"] as const,
  classes: ["classes"] as const,
  projects: ["projects"] as const,
  project: (slug: string) => ["project", slug] as const,
  bounds: (slug: string) => ["project", slug, "bounds"] as const,
  datasets: (slug: string) => ["project", slug, "datasets"] as const,
  jobs: (slug: string) => ["project", slug, "jobs"] as const,
  runs: (slug: string) => ["project", slug, "runs"] as const,
};

const forever = { staleTime: Number.POSITIVE_INFINITY };

export const useAnalyses = () =>
  useQuery({ queryKey: keys.analyses, queryFn: api.analyses, ...forever });
export const useClasses = () =>
  useQuery({ queryKey: keys.classes, queryFn: api.classes, ...forever });
export const useProjects = () => useQuery({ queryKey: keys.projects, queryFn: api.projects });
export const useProject = (slug: string) =>
  useQuery({ queryKey: keys.project(slug), queryFn: () => api.project(slug) });
export const useBounds = (slug: string) =>
  useQuery({ queryKey: keys.bounds(slug), queryFn: () => api.bounds(slug), ...forever });
export const useDatasets = (slug: string) =>
  useQuery({ queryKey: keys.datasets(slug), queryFn: () => api.datasets(slug) });
export const useJobs = (slug: string) =>
  useQuery({ queryKey: keys.jobs(slug), queryFn: () => api.jobs(slug) });
export const useRunList = (slug: string) =>
  useQuery({ queryKey: keys.runs(slug), queryFn: () => api.runs(slug) });
