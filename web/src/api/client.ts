// The HTTP API of `accessmod serve`, typed with the generated types.
import type { Json } from "../forms/schema";
import type { Api } from "../generated/types";

/** An error document of the API: what failed and every issue found. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly kind: string,
    readonly issues: string[],
  ) {
    super(issues.join("; ") || kind);
  }
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(path, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? null : JSON.stringify(body),
    signal: signal ?? null,
  });
  const text = await response.text();
  const value: unknown = text.length > 0 ? JSON.parse(text) : null;
  if (!response.ok) {
    const doc = (value ?? {}) as { kind?: string; issues?: string[] };
    throw new ApiError(response.status, doc.kind ?? "error", doc.issues ?? [response.statusText]);
  }
  return value as T;
}

const project = (slug: string) => `/api/projects/${encodeURIComponent(slug)}`;

export interface ClassInfo {
  class: Api.DataClass;
  kind: "raster" | "vector" | "table" | "config";
  label: string;
  importable: boolean;
}

export const api = {
  analyses: () => request<Api.AnalysisInfo[]>("GET", "/api/analyses"),
  classes: () => request<ClassInfo[]>("GET", "/api/classes"),
  projects: () => request<Api.ProjectEntry[]>("GET", "/api/projects"),
  project: (slug: string) => request<Api.ProjectInfo>("GET", project(slug)),
  bounds: (slug: string) =>
    request<[number, number, number, number]>("GET", `${project(slug)}/bounds`),
  datasets: (slug: string) => request<Api.Dataset[]>("GET", `${project(slug)}/datasets`),
  jobs: (slug: string) => request<Api.SavedJob[]>("GET", `${project(slug)}/jobs`),
  runs: (slug: string) => request<Api.RunRecord[]>("GET", `${project(slug)}/runs`),
  features: (slug: string, id: number, signal?: AbortSignal) =>
    request<GeoJSON.FeatureCollection & { truncated: boolean }>(
      "GET",
      `${project(slug)}/datasets/${id}/features`,
      undefined,
      signal,
    ),
  value: (slug: string, id: number, lon: number, lat: number) =>
    request<{ value: number | null }>(
      "GET",
      `${project(slug)}/datasets/${id}/value?lon=${lon}&lat=${lat}`,
    ),
  validate: (slug: string, analysis: string, document: Json, signal?: AbortSignal) =>
    request<Record<string, unknown>>(
      "POST",
      `${project(slug)}/analyses/${encodeURIComponent(analysis)}/validate`,
      document,
      signal,
    ),
  command: <T>(id: string, params: Record<string, unknown>) =>
    request<T>("POST", `/api/commands/${id}`, params),
  tileUrl: (slug: string, id: number) =>
    `${window.location.origin}${project(slug)}/datasets/${id}/tiles/{z}/{x}/{y}.png`,
};
