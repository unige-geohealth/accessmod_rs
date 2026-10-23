import { ApiError } from "../api/client";
import { t } from "../app/i18n";

/** Every issue of a failed request. */
export function Issues({ error }: { error: unknown }) {
  if (error === null || error === undefined) return null;
  const issues = error instanceof ApiError ? error.issues : [String(error)];
  return (
    <div className="issues" role="alert">
      <strong>{t("common.error")}</strong>
      <ul>
        {issues.map((issue) => (
          <li key={issue}>{issue}</li>
        ))}
      </ul>
    </div>
  );
}

export function Loading() {
  return <p className="muted">{t("common.loading")}</p>;
}

export function bytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function when(seconds: number | null | undefined): string {
  return seconds ? new Date(seconds * 1000).toLocaleString() : "";
}
