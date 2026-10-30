// The HTTP API of `accessmod serve`, typed with the generated types.
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

/** The issues of any error, for display. */
export function issuesOf(error: unknown): string[] {
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
  return value as T;
}

/** Commands the client sends: parameters and result. */
interface Commands {
  "project.create": [Api.CreateProject, Api.CreatedProject];
  "project.remove": [Api.ProjectRef, null];
  "dataset.import": [Api.ImportDataset, Api.Dataset];
  "dataset.tag": [Api.TagDataset, Api.Dataset];
  "dataset.remove": [Api.DatasetRef, null];
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
  projects: () => request<Api.ProjectEntry[]>("GET", "/api/projects"),
  project: (slug: string) => request<Api.ProjectInfo>("GET", project(slug)),
  /** `[west, south, east, north]` in WGS 84. */
  bounds: (slug: string) => request<[number, number, number, number]>("GET", `${project(slug)}/bounds`),
  datasets: (slug: string) => request<Api.Dataset[]>("GET", `${project(slug)}/datasets`),
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
      xhr.onerror = () => reject(new ApiError(0, "network", ["the upload failed: connection lost"]));
      xhr.onabort = () => reject(new ApiError(0, "cancelled", ["the upload was cancelled"]));
      signal?.addEventListener("abort", () => xhr.abort());
      xhr.send(form);
    }),
};
