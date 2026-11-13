// Selectors shared by the analysis tabs: datasets of the catalogue (AM5
// labels "(r) land cover merged [demo]") and fields of a vector dataset.
import { ComboboxField, SelectField } from "@accessmod/ui/am/form";
import { type Key, useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import { useDatasets, useProject, useRuns } from "../../api/queries";
import type { Api } from "../../generated/types";
import { type TagNote, tagSuggestions } from "../../state/catalogue";
import type { ZoneInput, ZoneMessage } from "../../state/zones";
import { useFmt } from "../common";

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

/** A zone layer with its identifier and label fields. */
export function ZoneInputs({
  label = "analysis_select_zone_vector",
  options,
  input,
  fields,
  onMessage,
}: {
  label?: Key;
  options: Options;
  input: ZoneInput;
  fields: readonly string[];
  onMessage: (message: ZoneMessage) => void;
}) {
  return (
    <>
      <DatasetSelect label={label} options={options} value={input.layer} onChange={(id) => onMessage({ type: "layer", id })} />
      <FieldSelect label="analysis_select_zone_id_integer" fields={fields} value={input.idField} onChange={(field) => onMessage({ type: "idField", field })} />
      <FieldSelect label="analysis_select_zone_name_text" fields={fields} value={input.labelField} onChange={(field) => onMessage({ type: "labelField", field })} />
    </>
  );
}

/** AM5 link-style buttons ("[All]", "[Previous]"). */
export const LINK = "px-1 text-link hover:text-link-hover hover:underline disabled:opacity-50 disabled:no-underline";

/** Previous / next over pages of `page` rows; nothing when one page holds them all. */
export function Pager({ offset, page, total, onOffset }: { offset: number; page: number; total: number; onOffset: (offset: number) => void }) {
  const { t } = useI18n();
  const f = useFmt();
  if (total <= page) return null;
  return (
    <div className="flex items-center gap-2 text-sm">
      <button type="button" className={LINK} disabled={offset === 0} onClick={() => onOffset(Math.max(0, offset - page))}>
        [{t("analysis.previous")}]
      </button>
      <span>{f("analysis.rows_page", offset + 1, Math.min(offset + page, total), total)}</span>
      <button type="button" className={LINK} disabled={offset + page >= total} onClick={() => onOffset(offset + page)}>
        [{t("analysis.next")}]
      </button>
    </div>
  );
}

/** Output tags of `analysis`, with the tag sets it already wrote or is writing
 * as suggestions, and the next free numbered one: pick, edit, compute. */
export function TagsField({
  project,
  analysis,
  label = "analysis_settings_add_tag",
  value,
  onChange,
}: {
  project: string;
  analysis: string;
  label?: Key;
  value: string;
  onChange: (text: string) => void;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const datasets = useDatasets(project).data ?? [];
  const runs = useRuns(project).data;
  const note = (n: TagNote) => (n.kind === "job" ? f("tags.suggest_job", n.run) : t(n.kind === "free" ? "tags.suggest_free" : "tags.suggest_exists"));
  const suggestions = tagSuggestions(analysis, value, datasets, runs).map((s) => ({ value: s.tags.join(" "), note: note(s.note) }));
  return <ComboboxField label={label} value={value} onChange={onChange} suggestions={suggestions} />;
}
