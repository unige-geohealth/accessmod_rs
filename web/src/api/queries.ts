// Server state: TanStack Query only. Queries are refreshed by events
// (events.ts) and after commands; nothing else writes them.
import { QueryClient, useQuery } from "@tanstack/react-query";
import type { Api } from "../generated/types";
import { api, type RowsQuery } from "./client";

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
  rows: (slug: string, id: number, q: RowsQuery) => ["project", slug, "rows", id, q] as const,
  values: (slug: string, id: number, field: string, label: string) => ["project", slug, "values", id, field, label] as const,
  features: (slug: string, id: number) => ["project", slug, "features", id] as const,
  value: (slug: string, id: number, lon: number, lat: number) => ["project", slug, "value", id, lon, lat] as const,
  jobs: (slug: string) => ["project", slug, "jobs"] as const,
  runs: (slug: string) => ["project", slug, "runs"] as const,
  allRuns: ["runs"] as const,
  /** Latest progress of each running run, written by events only. */
  progress: (slug: string) => ["project", slug, "progress"] as const,
};

/** Progress of the runs of a project, by run id. */
export type ProgressByRun = Readonly<Record<number, Api.Progress>>;

export const useHealth = () => useQuery({ queryKey: keys.health, queryFn: api.health, staleTime: Infinity });

export const useSettings = () => useQuery({ queryKey: keys.settings, queryFn: api.settings });

export const useProjects = () => useQuery({ queryKey: keys.projects, queryFn: api.projects });

export const useProject = (slug: string | null) =>
  useQuery({ queryKey: keys.project(slug ?? ""), queryFn: () => api.project(slug ?? ""), enabled: slug !== null });

export const useBounds = (slug: string | null) =>
  useQuery({ queryKey: keys.bounds(slug ?? ""), queryFn: () => api.bounds(slug ?? ""), enabled: slug !== null });

export const useDatasets = (slug: string | null) =>
  useQuery({ queryKey: keys.datasets(slug ?? ""), queryFn: () => api.datasets(slug ?? ""), enabled: slug !== null });

export const useRuns = (slug: string | null) =>
  useQuery({ queryKey: keys.runs(slug ?? ""), queryFn: () => api.runs(slug ?? ""), enabled: slug !== null });

/** Runs of every project (Jobs). */
export const useAllRuns = () => useQuery({ queryKey: keys.allRuns, queryFn: () => api.allRuns() });

export const useJobs = (slug: string | null) =>
  useQuery({ queryKey: keys.jobs(slug ?? ""), queryFn: () => api.jobs(slug ?? ""), enabled: slug !== null });

/** Progress is pushed by events, never fetched. */
export const useProgress = (slug: string | null) =>
  useQuery<ProgressByRun>({
    queryKey: keys.progress(slug ?? ""),
    queryFn: () => queryClient.getQueryData<ProgressByRun>(keys.progress(slug ?? "")) ?? {},
    enabled: slug !== null,
    staleTime: Infinity,
  });

/** A page of rows of a dataset; `id` `null` disables the query. */
export const useRows = (slug: string, id: number | null, q: RowsQuery) =>
  useQuery({
    queryKey: keys.rows(slug, id ?? 0, q),
    queryFn: ({ signal }) => api.rows(slug, id ?? 0, q, signal),
    enabled: id !== null,
    placeholderData: (previous) => previous,
  });

/** Distinct values of `field` of a vector dataset; disabled without a dataset or field. */
export const useValues = (slug: string, id: number | null, field: string, label: string) =>
  useQuery({
    queryKey: keys.values(slug, id ?? 0, field, label),
    queryFn: ({ signal }) => api.values(slug, id ?? 0, field, label, signal),
    enabled: id !== null && field !== "",
    // A dataset never changes once imported: one read per field.
    staleTime: Infinity,
  });

/** Features of a vector dataset (GeoJSON, WGS 84); disabled without a dataset. */
export const useFeatures = (slug: string, id: number | null) =>
  useQuery({
    queryKey: keys.features(slug, id ?? 0),
    queryFn: ({ signal }) => api.features(slug, id ?? 0, signal),
    enabled: id !== null,
  });

/** Value of a raster dataset at a point; disabled without a dataset or point. */
export const useValue = (slug: string, id: number | null, point: { lon: number; lat: number } | null) =>
  useQuery({
    queryKey: keys.value(slug, id ?? 0, point?.lon ?? 0, point?.lat ?? 0),
    queryFn: ({ signal }) => api.value(slug, id ?? 0, point?.lon ?? 0, point?.lat ?? 0, signal),
    enabled: id !== null && point !== null,
  });

/** Server validation (`prepare`) of a complete job document; `null` disables it. */
export const useValidation = (slug: string, analysis: string, document: unknown) => {
  const text = document === null ? null : JSON.stringify(document);
  return useQuery({
    queryKey: ["project", slug, "validate", analysis, text] as const,
    queryFn: ({ signal }) => api.validate(slug, analysis, document, signal),
    enabled: text !== null,
    retry: false,
    staleTime: 60_000,
  });
};
