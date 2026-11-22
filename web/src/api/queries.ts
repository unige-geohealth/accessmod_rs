// Server state: TanStack Query only. Queries are refreshed by events
// (events.ts) and after commands; nothing else writes them.
import { QueryClient, useQueries, useQuery } from "@tanstack/react-query";
import type { Api } from "../generated/types";
import { api, type RowsQuery } from "./client";

export const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false, staleTime: 30_000 } },
});

export const keys = {
  health: ["health"] as const,
  settings: ["settings"] as const,
  session: ["session"] as const,
  /** Size of the Project Creator download cache. */
  creatorCache: ["creator-cache"] as const,
  projects: ["projects"] as const,
  project: (slug: string) => ["project", slug] as const,
  bounds: (slug: string) => ["project", slug, "bounds"] as const,
  datasets: (slug: string) => ["project", slug, "datasets"] as const,
  rows: (slug: string, id: number, q: RowsQuery) => ["project", slug, "rows", id, q] as const,
  values: (slug: string, id: number, field: string, label: string) => ["project", slug, "values", id, field, label] as const,
  features: (slug: string, id: number, sample: number | null) => ["project", slug, "features", id, sample] as const,
  basemap: ["basemap"] as const,
  value: (slug: string, id: number, lon: number, lat: number) => ["project", slug, "value", id, lon, lat] as const,
  jobs: (slug: string) => ["project", slug, "jobs"] as const,
  archives: (slug: string) => ["project", slug, "archives"] as const,
  runs: (slug: string) => ["project", slug, "runs"] as const,
  allRuns: ["runs"] as const,
  /** Tasks waiting or running in the server's lanes (events keep it current). */
  queue: ["queue"] as const,
  /** Operations making projects (events keep it current). */
  operations: ["operations"] as const,
  /** Latest progress of each running run, written by events only. */
  progress: (slug: string) => ["project", slug, "progress"] as const,
};

/** Progress of the runs of a project, by run id. */
export type ProgressByRun = Readonly<Record<number, Api.Progress>>;

export const useHealth = () => useQuery({ queryKey: keys.health, queryFn: api.health, staleTime: Infinity });

export const useSettings = () => useQuery({ queryKey: keys.settings, queryFn: api.settings });

/** Who the server sees (an administrator or not). */
export const useSession = () => useQuery({ queryKey: keys.session, queryFn: api.session, staleTime: Infinity });

/** Size of the Project Creator download cache (administrators only). */
export const useCreatorCache = () => useQuery({ queryKey: keys.creatorCache, queryFn: () => api.command("creator.cache", {}) });

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

export const useArchives = (slug: string | null) =>
  useQuery({ queryKey: keys.archives(slug ?? ""), queryFn: () => api.archives(slug ?? ""), enabled: slug !== null });

export const useJobs = (slug: string | null) =>
  useQuery({ queryKey: keys.jobs(slug ?? ""), queryFn: () => api.jobs(slug ?? ""), enabled: slug !== null });

/** Tasks of the lanes, read once then kept current by events. */
export const useQueue = () => useQuery({ queryKey: keys.queue, queryFn: () => api.queue(), staleTime: Infinity });

/** Operations making projects, read once then kept current by events. */
export const useOperations = () => useQuery({ queryKey: keys.operations, queryFn: () => api.operations(), staleTime: Infinity });

/** Progress of the runs of project `slug`: pushed by events, never fetched. */
const progressQuery = (slug: string) => ({
  queryKey: keys.progress(slug),
  queryFn: () => queryClient.getQueryData<ProgressByRun>(keys.progress(slug)) ?? {},
  staleTime: Infinity,
});

export const useProgress = (slug: string | null) => useQuery<ProgressByRun>({ ...progressQuery(slug ?? ""), enabled: slug !== null });

/** Progress of the runs of several projects, by project. */
export const useProgressOf = (slugs: readonly string[]): Readonly<Record<string, ProgressByRun>> =>
  useQueries({
    queries: slugs.map((slug) => progressQuery(slug)),
    combine: (results) => Object.fromEntries(slugs.map((slug, i) => [slug, results[i]?.data ?? {}])),
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

/** Features of a vector dataset (GeoJSON, WGS 84), each with the value of raster `sample` at its point; disabled without a dataset. */
export const useFeatures = (slug: string, id: number | null, sample: number | null = null) =>
  useQuery({
    queryKey: keys.features(slug, id ?? 0, sample),
    queryFn: ({ signal }) => api.features(slug, id ?? 0, sample, signal),
    enabled: id !== null,
    // Datasets never change once imported.
    staleTime: Infinity,
  });

/** The server's MapTiler key (online basemaps). */
export const useBasemap = () => useQuery({ queryKey: keys.basemap, queryFn: () => api.basemap(), staleTime: Infinity });

/** Countries a project can be created in (Project Creator). */
export const useCountries = () => useQuery({ queryKey: ["creator", "countries"], queryFn: () => api.command("creator.countries", {}), staleTime: Infinity, retry: 1 });

/** Units of `iso3` at `level`, simplified (Project Creator). */
export const useUnits = (iso3: string | null, level: number) =>
  useQuery({
    queryKey: ["creator", "units", iso3, level],
    queryFn: () => api.command("creator.units", { iso3: iso3 ?? "", level }),
    enabled: iso3 !== null,
    staleTime: Infinity,
    retry: false,
  });

/** Value of a raster dataset at a point; disabled without a dataset or point. */
export const useValue = (slug: string, id: number | null, point: { lon: number; lat: number } | null) =>
  useQuery({
    queryKey: keys.value(slug, id ?? 0, point?.lon ?? 0, point?.lat ?? 0),
    queryFn: ({ signal }) => api.value(slug, id ?? 0, point?.lon ?? 0, point?.lat ?? 0, signal),
    enabled: id !== null && point !== null,
  });

/** The quick check of a complete job document: types, paths, classes, then
 * the analysis's lint (facilities read one cell each, class counts stored at
 * import), never a whole grid. Its warnings; `null` disables it. */
export const useQuickCheck = (slug: string, analysis: string, document: unknown) => {
  const text = document === null ? null : JSON.stringify(document);
  return useQuery({
    queryKey: ["project", slug, "quick-check", analysis, text] as const,
    queryFn: ({ signal }) => api.check(slug, analysis, document, signal),
    enabled: text !== null,
    retry: false,
    staleTime: 60_000,
  });
};

