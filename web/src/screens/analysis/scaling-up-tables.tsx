// Analysis › Scaling up tables: the forms adding a suitability factor or an
// exclusion area (AM5 "Scaling up tables" card), and the capacity,
// suitability and exclusion tables (AM5 "Scaling up" section).
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { RadioField, SelectField, TextField } from "@accessmod/ui/am/form";
import { AmButton, LinkActions } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { CenterTitle, SubTitle } from "@accessmod/ui/am/section";
import { type Key, useI18n } from "@accessmod/ui/i18n";
import { useState } from "react";
import schema from "../../../schemas/jobs/scaling-up.json";
import type { Api } from "../../generated/types";
import { datasetLabel, datasetOptions } from "../../state/catalogue";
import type { ScalingUpMessage } from "../../state/scaling-up";
import {
  type CapacityRow,
  DYNAMIC,
  editCapacity,
  type ExclusionRow,
  FACTORS,
  type FactorKind,
  type FactorRow,
  factorOptions,
  type LayerRef,
  NEW_FACTOR,
  nextId,
} from "../../state/scaling-up-tables";

type Dispatch = (message: ScalingUpMessage) => void;

/** Job names of the AM5 factors. */
const JOB_FACTOR: Record<FactorKind, string> = { popsum: "population", dist: "distance", traveltime: "travel_time", priority: "priority" };

const FACTOR_LABELS: Record<FactorKind, Key> = {
  popsum: "analysis_scaleup_select_factor_popsum",
  dist: "analysis_scaleup_select_factor_dist",
  traveltime: "analysis_scaleup_select_factor_traveltime",
  priority: "analysis_scaleup_select_factor_priority",
};

interface LayerSpec {
  classes: Api.DataClass[];
  dynamic: ("facilities" | "population")[];
}

/** Layer classes and dynamic layers of a rule, from the job schema. */
function layerSpec(properties: Record<string, Record<string, unknown>>): LayerSpec {
  const layer = properties.layer ?? {};
  return {
    classes: (layer["x-accessmod-class"] as Api.DataClass[] | undefined) ?? [],
    dynamic: (layer["x-accessmod-dynamic"] as LayerSpec["dynamic"] | undefined) ?? [],
  };
}

type Defs = { $defs: { Factor: { oneOf: { properties: Record<string, Record<string, unknown>> }[] }; Exclusion: { properties: Record<string, Record<string, unknown>> } } };
const DEFS = (schema as unknown as Defs).$defs;
const FACTOR_LAYERS = Object.fromEntries(
  FACTORS.map((f) => [f, layerSpec(DEFS.Factor.oneOf.find((o) => o.properties.factor?.const === JOB_FACTOR[f])?.properties ?? {})]),
) as Record<FactorKind, LayerSpec>;
const EXCLUSION_LAYERS = layerSpec(DEFS.Exclusion.properties);

/** Select options of a rule layer: the dynamic layers first (AM5 preselects them), then datasets. */
function layerOptions(t: (key: Key) => string, datasets: readonly Api.Dataset[] | undefined, spec: LayerSpec) {
  return [...spec.dynamic.map((d) => ({ value: d, label: t(DYNAMIC[d]) })), ...datasetOptions(t, datasets, spec.classes)];
}

const toLayer = (value: string): LayerRef => (value === "facilities" || value === "population" ? value : { dataset: Number(value) });

/** AM5 label of a rule layer. */
function layerLabel(t: (key: Key) => string, layer: LayerRef, byId: ReadonlyMap<number, Api.Dataset>): string {
  if (typeof layer === "string") return t(DYNAMIC[layer]);
  if ("missing" in layer) return layer.missing;
  const d = byId.get(layer.dataset);
  return d ? datasetLabel(t, d) : `#${layer.dataset}`;
}

/** Number typed in a form, or `undefined`. */
const numberOf = (text: string): number | undefined => (text.trim() === "" || !Number.isFinite(Number(text)) ? undefined : Number(text));

/** "Scaling up tables" card: forms adding a factor or an exclusion. */
export function RuleForms({
  datasets,
  factors,
  exclusions,
  dispatch,
}: {
  datasets: readonly Api.Dataset[] | undefined;
  factors: readonly FactorRow[];
  exclusions: readonly ExclusionRow[];
  dispatch: Dispatch;
}) {
  const { t } = useI18n();
  // Form inputs before "Add": view state, not part of the draft.
  const [factor, setFactor] = useState({ ...NEW_FACTOR, layer: "", weight: "1", radius: "1" });
  const [exclusion, setExclusion] = useState({ layer: "", buffer: "5", method: "keep_inside" as ExclusionRow["method"] });
  const factorLayers = layerOptions(t, datasets, FACTOR_LAYERS[factor.factor]);
  const factorLayer = factorLayers.some((o) => o.value === factor.layer) ? factor.layer : (factorLayers[0]?.value ?? "");
  const exclusionLayers = layerOptions(t, datasets, EXCLUSION_LAYERS);
  const exclusionLayer = exclusionLayers.some((o) => o.value === exclusion.layer) ? exclusion.layer : (exclusionLayers[0]?.value ?? "");
  const weight = numberOf(factor.weight);
  const radius = numberOf(factor.radius);
  const buffer = numberOf(exclusion.buffer);

  const addFactor = () => {
    if (factorLayer === "" || weight === undefined || radius === undefined) return;
    const { layer: _, weight: __, radius: ___, ...rest } = factor;
    dispatch({ type: "factors", rows: [...factors, { ...rest, id: nextId(factors), layer: toLayer(factorLayer), weight, radiusKm: radius }] });
  };
  const addExclusion = () => {
    if (exclusionLayer === "" || buffer === undefined) return;
    dispatch({
      type: "exclusions",
      rows: [...exclusions, { id: nextId(exclusions), layer: toLayer(exclusionLayer), bufferKm: buffer, method: exclusion.method }],
    });
  };

  return (
    <PanelAccordion
      items={[
        {
          value: "suitability",
          title: "analysis_scaleup_suitability_index_title",
          content: (
            <div className="flex flex-col gap-[15px]">
              <SelectField
                label="analysis_scaleup_select_factor"
                options={FACTORS.map((f) => ({ value: f, label: t(FACTOR_LABELS[f]) }))}
                value={factor.factor}
                onChange={(v) => setFactor({ ...factor, factor: v as FactorKind })}
              />
              {factor.factor === "popsum" && (
                <TextField label="analysis_scaleup_set_radius" type="number" value={factor.radius} onChange={(radius) => setFactor({ ...factor, radius })} />
              )}
              {factor.factor === "traveltime" && (
                <>
                  <RadioField
                    label="analysis_scaleup_type"
                    value={factor.method}
                    onChange={(v) => setFactor({ ...factor, method: v as FactorRow["method"] })}
                    options={[
                      { value: "iso", label: "analysis_settings_isotropic" },
                      { value: "aniso", label: "analysis_settings_anisotropic" },
                    ]}
                  />
                  {factor.method === "aniso" && (
                    <RadioField
                      label="analysis_scaleup_travel_direction"
                      value={factor.direction}
                      onChange={(v) => setFactor({ ...factor, direction: v as FactorRow["direction"] })}
                      options={[
                        { value: "from", label: "analysis_settings_travel_from_hf" },
                        { value: "to", label: "analysis_settings_travel_to_hf" },
                      ]}
                    />
                  )}
                  <RadioField
                    label="analysis_scaleup_knight_move"
                    value={factor.knight ? "n16" : "n8"}
                    onChange={(v) => setFactor({ ...factor, knight: v === "n16" })}
                    options={[
                      { value: "n8", label: "analysis_settings_knight_move_no" },
                      { value: "n16", label: "analysis_settings_knight_move_yes" },
                    ]}
                  />
                </>
              )}
              <RadioField
                label="analysis_scaleup_priority"
                value={factor.prefer}
                onChange={(v) => setFactor({ ...factor, prefer: v as FactorRow["prefer"] })}
                options={[
                  { value: "hvms", label: "analysis_scaleup_priority_hvms" },
                  { value: "hvls", label: "analysis_scaleup_priority_hvls" },
                ]}
              />
              <TextField label="analysis_scaleup_select_weight" type="number" value={factor.weight} onChange={(w) => setFactor({ ...factor, weight: w })} />
              <SelectField
                label="analysis_scaleup_select_layer"
                options={factorLayers}
                value={factorLayer || undefined}
                onChange={(layer) => setFactor({ ...factor, layer })}
              />
              <AmButton k="analysis_scaleup_add_btn" disabled={factorLayer === "" || weight === undefined || radius === undefined} onClick={addFactor} />
            </div>
          ),
        },
        {
          value: "exclusion",
          title: "analysis_scaleup_exclusion_main",
          content: (
            <div className="flex flex-col gap-[15px]">
              <SelectField
                label="analysis_scaleup_select_exclusion"
                options={exclusionLayers}
                value={exclusionLayer || undefined}
                onChange={(layer) => setExclusion({ ...exclusion, layer })}
              />
              <TextField
                label="analysis_scaleup_optional_buffer"
                type="number"
                value={exclusion.buffer}
                onChange={(b) => setExclusion({ ...exclusion, buffer: b })}
              />
              <RadioField
                label="analysis_scaleup_exclusion_method"
                value={exclusion.method}
                onChange={(v) => setExclusion({ ...exclusion, method: v as ExclusionRow["method"] })}
                options={[
                  { value: "keep_inside", label: "analysis_scaleup_exclusion_method_keep_inside" },
                  { value: "keep_outside", label: "analysis_scaleup_exclusion_method_keep_outside" },
                ]}
              />
              <AmButton k="analysis_scaleup_exclusion_add_btn" disabled={exclusionLayer === "" || buffer === undefined} onClick={addExclusion} />
            </div>
          ),
        },
      ]}
    />
  );
}

const CAPACITY_COLUMNS: Column[] = [
  { field: "min", editable: "number" },
  { field: "max", editable: "number" },
  { field: "label", editable: true },
  { field: "capacity", editable: "number" },
];
const FACTOR_COLUMNS: Column[] = ["factor", "layer", "weight", "options"].map((field) => ({ field }));
const EXCLUSION_COLUMNS: Column[] = ["layer", "buffer", "method"].map((field) => ({ field }));

/** AM5 "Scaling up" section: capacity, suitability and exclusion tables. */
export function ScalingUpTables({
  capacities,
  factors,
  exclusions,
  byId,
  dispatch,
}: {
  capacities: readonly CapacityRow[];
  factors: readonly FactorRow[];
  exclusions: readonly ExclusionRow[];
  byId: ReadonlyMap<number, Api.Dataset>;
  dispatch: Dispatch;
}) {
  const { t } = useI18n();
  // Rows checked for removal: view state.
  const [checkedFactors, setCheckedFactors] = useState<number[]>([]);
  const [checkedExclusions, setCheckedExclusions] = useState<number[]>([]);
  const ids = (rows: Row[]) => rows.map((r) => Number(r.id));
  return (
    <>
      <CenterTitle title="analysis_table_scaleup_title" sub="analysis_table_scaleup_sub" />
      <div className="px-[15px]">
        <SubTitle k="analysis_table_scaleup_new_facility" />
        <LinkActions
          actions={[
            {
              icon: "add",
              k: "analysis_table_scaleup_add_row_btn",
              onClick: () => dispatch({ type: "capacities", rows: [...capacities, { min: 0, max: 0, label: "label", capacity: 0 }] }),
            },
            { icon: "remove", k: "analysis_table_scaleup_rm_row_btn", onClick: () => dispatch({ type: "capacities", rows: capacities.slice(0, -1) }) },
          ]}
        />
        <DataTable
          toolbar={false}
          columns={CAPACITY_COLUMNS}
          data={capacities.map((r, i) => ({ ...r, row: i }))}
          rowId="row"
          height="20vh"
          onCellEdited={(row, field, value) => dispatch({ type: "capacities", rows: editCapacity(capacities, Number(row.row), field, value) })}
        />

        <SubTitle k="analysis_table_scaleup_suitability" />
        <LinkActions
          actions={[
            { icon: "undo", k: "analysis_table_scaleup_reset_suitability_btn", onClick: () => dispatch({ type: "reset", table: "factors" }) },
            {
              icon: "remove",
              k: "analysis_table_scaleup_rm_selected_suitability_btn",
              onClick: () => dispatch({ type: "factors", rows: factors.filter((r) => !checkedFactors.includes(r.id)) }),
            },
          ]}
        />
        <DataTable
          toolbar={false}
          selectable
          rowId="id"
          selected={checkedFactors.filter((id) => factors.some((r) => r.id === id))}
          onSelectionChange={(rows) => setCheckedFactors(ids(rows))}
          columns={FACTOR_COLUMNS}
          data={factors.map((r) => ({ id: r.id, factor: r.factor, layer: layerLabel(t, r.layer, byId), weight: r.weight, options: factorOptions(r) }))}
          height="20vh"
        />

        <SubTitle k="analysis_table_scaleup_exclusion" />
        <LinkActions
          actions={[
            { icon: "undo", k: "analysis_table_scaleup_reset_exclusion_btn", onClick: () => dispatch({ type: "reset", table: "exclusions" }) },
            {
              icon: "remove",
              k: "analysis_table_scaleup_rm_exclusion_btn",
              onClick: () => dispatch({ type: "exclusions", rows: exclusions.filter((r) => !checkedExclusions.includes(r.id)) }),
            },
          ]}
        />
        <DataTable
          toolbar={false}
          selectable
          rowId="id"
          selected={checkedExclusions.filter((id) => exclusions.some((r) => r.id === id))}
          onSelectionChange={(rows) => setCheckedExclusions(ids(rows))}
          columns={EXCLUSION_COLUMNS}
          data={exclusions.map((r) => ({ id: r.id, layer: layerLabel(t, r.layer, byId), buffer: r.bufferKm, method: r.method }))}
          height="20vh"
        />
      </div>
    </>
  );
}
