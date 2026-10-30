// Selectors shared by the analysis tabs: datasets of the catalogue (AM5
// labels "(r) land cover merged [demo]") and fields of a vector dataset.
import { SelectField } from "@accessmod/ui/am/form";
import type { Key } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import { useDatasets, useProject } from "../../api/queries";
import type { Api } from "../../generated/types";

/** Datasets of a project by id, and its DEM. */
export function useCatalogue(project: string) {
  const datasets = useDatasets(project).data;
  const dem = useProject(project).data?.dem ?? null;
  const byId = useMemo(() => new Map((datasets ?? []).map((d) => [d.id, d])), [datasets]);
  return { datasets, byId, dem };
}

export type Options = { value: string; label: string }[];

/** First option (AM5 preselects the newest layer of a class). */
export const firstOf = (options: Options): number | null => (options[0] ? Number(options[0].value) : null);

/** Attribute fields of a vector dataset. */
export function fieldsOf(byId: ReadonlyMap<number, Api.Dataset>, id: number | null): readonly string[] {
  const summary = id === null ? undefined : byId.get(id)?.summary;
  return summary?.kind === "vector" ? summary.fields : [];
}

/** A dataset select; choosing what is shown changes nothing (defaults included). */
export function DatasetSelect({
  label,
  options,
  value,
  onChange,
}: {
  label: Key;
  options: Options;
  value: number | null;
  onChange: (id: number | null) => void;
}) {
  return (
    <SelectField
      label={label}
      options={options}
      value={value === null ? undefined : String(value)}
      onChange={(v) => {
        const id = v === "" ? null : Number(v);
        if (id !== value) onChange(id);
      }}
    />
  );
}

/** A field of a vector dataset, indented under its layer. */
export function FieldSelect({ label, fields, value, onChange }: { label: Key; fields: readonly string[]; value: string; onChange: (field: string) => void }) {
  if (fields.length === 0) return null;
  return (
    <SelectField
      indent
      label={label}
      options={fields.map((f) => ({ value: f, label: f }))}
      value={value}
      onChange={(field) => {
        if (field !== value) onChange(field);
      }}
    />
  );
}
