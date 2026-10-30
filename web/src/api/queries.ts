// Server state: TanStack Query only. Queries are refreshed by events
// (events.ts) and after commands; nothing else writes them.
import { QueryClient, useQuery } from "@tanstack/react-query";
import { api } from "./client";

export const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false, staleTime: 30_000 } },
});

export const keys = {
  health: ["health"] as const,
  settings: ["settings"] as const,
  projects: ["projects"] as const,
  project: (slug: string) => ["project", slug] as const,
  bounds: (slug: string) => ["project", slug, "bounds"] as const,
  datasets: (slug: string) => ["project", slug, "datasets"] as const,
};

export const useHealth = () => useQuery({ queryKey: keys.health, queryFn: api.health, staleTime: Infinity });

export const useSettings = () => useQuery({ queryKey: keys.settings, queryFn: api.settings });

export const useProjects = () => useQuery({ queryKey: keys.projects, queryFn: api.projects });

export const useProject = (slug: string | null) =>
  useQuery({ queryKey: keys.project(slug ?? ""), queryFn: () => api.project(slug ?? ""), enabled: slug !== null });

export const useBounds = (slug: string | null) =>
  useQuery({ queryKey: keys.bounds(slug ?? ""), queryFn: () => api.bounds(slug ?? ""), enabled: slug !== null });

export const useDatasets = (slug: string | null) =>
  useQuery({ queryKey: keys.datasets(slug ?? ""), queryFn: () => api.datasets(slug ?? ""), enabled: slug !== null });
