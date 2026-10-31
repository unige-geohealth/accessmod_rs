// The HTTP API of `accessmod serve`, typed with the generated types.
import type { Key } from "@accessmod/ui/i18n";
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

/** Errors the client makes itself, by kind: their text is translated. */
const OWN: Record<string, Key> = {
  network: "upload.connection_lost",
  cancelled: "upload.cancelled",
  not_json: "api.not_json",
};

/** The issues of any error, for display; server issues are shown as sent. */
export function issuesOf(error: unknown, t: (key: Key) => string): string[] {
  if (error instanceof ApiError && error.status === 0) {
    const key = OWN[error.kind];
    if (key) return [t(key)];
  }
  if (error instanceof ApiError) return error.issues;
  if (error instanceof Error) return [error.message];
  return [String(error)];
}

function apiError(status: number, statusText: string, value: unknown): ApiError {
  const doc = (value ?? {}) as { kind?: string; issues?: string[] };
  return new ApiError(status, doc.kind ?? "error", doc.issues ?? [statusText || `HTTP ${status}`]);
}

function parse(text: string): unknown {
  if (text.length === 0) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

async function request<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const response = await fetch(path, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? null : JSON.stringify(body),
    signal: signal ?? null,
  });
  const value = parse(await response.text());
  if (!response.ok) throw apiError(response.status, response.statusText, value);
  // A page instead of a document: a server older than the client.
  if (typeof value === "string") throw new ApiError(0, "not_json", []);
  return value as T;
}

/** Commands the client sends: parameters and result. */
interface Commands {
  "project.create": [Api.CreateProject, Api.CreatedProject];
  "project.remove": [Api.ProjectRef, null];
  "demo.reset": [Record<string, never>, Api.CreatedProject];
  "settings.default_project": [Api.DefaultProject, Api.Settings];
  "dataset.import": [Api.ImportDataset, Api.Dataset];
  "dataset.tag": [Api.TagDataset, Api.Dataset];
  "dataset.remove": [Api.DatasetRef, null];
  "job.save": [Api.SaveJob, Api.SavedJob];
  "job.remove": [Api.JobRef, null];
  "run.start": [Api.StartRun, Api.RunRecord];
  "run.cancel": [Api.RunRef, Api.RunRecord];
}

/** Query of `GET …/datasets/{id}/rows`. */
export interface RowsQuery {
  offset?: number;
  limit?: number;
  /** OGR SQL attribute filter (vector datasets). */
  where?: string;
  /** Raster datasets read at the point of each row. */
  sample?: number[];
}

function rowsSearch(q: RowsQuery): string {
  const params = new URLSearchParams();
  if (q.offset) params.set("offset", String(q.offset));
  if (q.limit !== undefined) params.set("limit", String(q.limit));
  if (q.where && q.where.trim() !== "") params.set("where", q.where);
  if (q.sample && q.sample.length > 0) params.set("sample", q.sample.join(","));
  const search = params.toString();
  return search === "" ? "" : `?${search}`;
}

/** A data class as `GET /api/classes` (and `web/schemas/data-classes.json`) lists it. */
export interface ClassInfo {
  class: Api.DataClass;
  kind: "raster" | "vector" | "table" | "config";
  label: string;
  importable: boolean;
}

const project = (slug: string) => `/api/projects/${encodeURIComponent(slug)}`;

export const api = {
  health: () => request<{ status: string; version: string }>("GET", "/api/health"),
  settings: () => request<Api.Settings>("GET", "/api/settings"),
  projects: () => request<Api.ProjectEntry[]>("GET", "/api/projects"),
  project: (slug: string) => request<Api.ProjectInfo>("GET", project(slug)),
  /** `[west, south, east, north]` in WGS 84. */
  bounds: (slug: string) => request<[number, number, number, number]>("GET", `${project(slug)}/bounds`),
  datasets: (slug: string) => request<Api.Dataset[]>("GET", `${project(slug)}/datasets`),
  rows: (slug: string, id: number, q: RowsQuery, signal?: AbortSignal) =>
    request<Api.Rows>("GET", `${project(slug)}/datasets/${id}/rows${rowsSearch(q)}`, undefined, signal),
  /** Distinct values of a field of a vector dataset, with the label of each. */
  values: (slug: string, id: number, field: string, label: string, signal?: AbortSignal) =>
    request<Api.FieldValues>(
      "GET",
      `${project(slug)}/datasets/${id}/values?${new URLSearchParams(label === "" ? { field } : { field, label }).toString()}`,
      undefined,
      signal,
    ),
  /** URL of a table dataset as a file. */
  downloadUrl: (slug: string, id: number, format: "csv" | "json") => `${project(slug)}/datasets/${id}/download?format=${format}`,
  jobs: (slug: string) => request<Api.SavedJob[]>("GET", `${project(slug)}/jobs`),
  runs: (slug: string) => request<Api.RunRecord[]>("GET", `${project(slug)}/runs`),
  /** The latest runs of every project, newest first. */
  allRuns: () => request<Api.JobEntry[]>("GET", "/api/runs"),
  /** `prepare` on a job document: its validation summary, or an error with every issue. */
  validate: (slug: string, analysis: string, document: unknown, signal?: AbortSignal) =>
    request<Record<string, unknown>>("POST", `${project(slug)}/analyses/${encodeURIComponent(analysis)}/validate`, document, signal),
  command: <K extends keyof Commands>(id: K, params: Commands[K][0]) =>
    request<Commands[K][1]>("POST", `/api/commands/${id}`, params),

  /**
   * `POST /api/uploads`: the files as one multipart upload. XMLHttpRequest,
   * since fetch reports no upload progress; `onProgress` gets 0 to 1.
   */
  upload: (files: File[], onProgress: (fraction: number) => void, signal?: AbortSignal) =>
    new Promise<Api.Upload>((resolve, reject) => {
      const form = new FormData();
      for (const file of files) form.append("file", file, file.name);
      const xhr = new XMLHttpRequest();
      xhr.open("POST", "/api/uploads");
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable && e.total > 0) onProgress(e.loaded / e.total);
      };
      xhr.onload = () => {
        const value = parse(xhr.responseText);
        if (xhr.status >= 200 && xhr.status < 300) resolve(value as Api.Upload);
        else reject(apiError(xhr.status, xhr.statusText, value));
      };
      xhr.onerror = () => reject(new ApiError(0, "network", []));
      xhr.onabort = () => reject(new ApiError(0, "cancelled", []));
      signal?.addEventListener("abort", () => xhr.abort());
      xhr.send(form);
    }),
};
