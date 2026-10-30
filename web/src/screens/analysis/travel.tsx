// The travel part of an analysis tab (Accessibility, Geographic coverage,
// Referral, Scaling up): land cover and scenario inputs, travel settings and
// the "Travel scenario" tables.
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { CheckboxField, RadioField, TextField } from "@accessmod/ui/am/form";
import { LinkActions, type Message, MessageList } from "@accessmod/ui/am/messages";
import { CenterTitle, SubTitle } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import { useRows } from "../../api/queries";
import type { Api } from "../../generated/types";
import { datasetOptions, pairedTable } from "../../state/catalogue";
import { editRow, initialRows, labelsOf, landCoverClasses, MODES, mergeTable, tableNotices, tableRows } from "../../state/scenario";
import { scenarioRows, type TravelDraft, type TravelMessage, withTravelDefaults } from "../../state/travel";
import { useFmt } from "../common";
import { DatasetSelect, firstOf, useCatalogue } from "./inputs";

/** Rows of a table read whole (land cover labels, scenario tables): small by nature. */
export const WHOLE = { limit: 1000 };

/** The draft as shown (defaults applied), its options and its scenario rows. */
export function useTravel<D extends TravelDraft>(project: string, stored: D, classes: readonly Api.DataClass[]) {
  const { t } = useI18n();
  const { datasets, byId } = useCatalogue(project);
  const landCoverOptions = datasetOptions(t, datasets, classes);
  const scenarioOptions = datasetOptions(t, datasets, ["tScenario"]);
  const draft = withTravelDefaults(stored, firstOf(landCoverOptions));
  const scenarioTable = stored.scenarioTable ?? firstOf(scenarioOptions);
  const landCover = draft.landCover === null ? undefined : byId.get(draft.landCover);
  const labels = useRows(project, pairedTable(datasets, landCover, "tLandCover")?.id ?? null, WHOLE);
  const initial = useMemo(() => initialRows(landCoverClasses(landCover), labelsOf(labels.data)), [landCover, labels.data]);
  const scenario = useMemo(() => scenarioRows(draft, initial), [draft, initial]);
  return { draft, landCoverOptions, scenarioOptions, scenarioTable, hasLandCover: landCover !== undefined, scenario };
}

export type Travel = ReturnType<typeof useTravel>;

/** Data input: the merged land cover and the scenario table. */
export function TravelInputs({ travel, dispatch }: { travel: Travel; dispatch: (message: TravelMessage) => void }) {
  return (
    <>
      <DatasetSelect
        label="analysis_select_merged_lc_raster"
        options={travel.landCoverOptions}
        value={travel.draft.landCover}
        onChange={(id) => dispatch({ type: "landCover", id })}
      />
      <DatasetSelect
        label="analysis_select_scenario_table"
        options={travel.scenarioOptions}
        value={travel.scenarioTable}
        onChange={(id) => dispatch({ type: "scenarioTable", id })}
      />
    </>
  );
}

/** Analysis settings: method, direction (when the analysis has one), knight's move, maximum travel time. */
export function TravelSettings({ draft, dispatch, direction = true }: { draft: TravelDraft; dispatch: (message: TravelMessage) => void; direction?: boolean }) {
  return (
    <>
      <RadioField
        label="analysis_settings_type"
        value={draft.method}
        onChange={(v) => dispatch({ type: "method", method: v as TravelDraft["method"] })}
        options={[
          { value: "isotropic", label: "analysis_settings_isotropic" },
          { value: "anisotropic", label: "analysis_settings_anisotropic" },
        ]}
      />
      {direction && (
        <RadioField
          label="analysis_settings_travel_direction"
          value={draft.direction}
          onChange={(v) => dispatch({ type: "direction", direction: v as TravelDraft["direction"] })}
          options={[
            { value: "from_facilities", label: "analysis_settings_travel_from_hf" },
            { value: "towards_facilities", label: "analysis_settings_travel_to_hf" },
          ]}
        />
      )}
      <CheckboxField
        label="analysis_settings_knight_move"
        help="analysis_settings_knight_move_desc"
        checked={draft.knight}
        onChange={(on) => dispatch({ type: "knight", on })}
      />
      <TextField label="analysis_settings_max_travel_time" type="number" value={draft.minutes} onChange={(text) => dispatch({ type: "minutes", text })} />
    </>
  );
}

const SCENARIO_COLUMNS: Column[] = [
  { field: "class" },
  { field: "label" },
  { field: "speed", editable: "number" },
  { field: "mode", editable: { values: [...MODES] } },
];

/** "Travel scenario": the rows to process (editable) and the selected scenario table. */
export function ScenarioSection({ project, travel, dispatch }: { project: string; travel: Travel; dispatch: (message: TravelMessage) => void }) {
  const f = useFmt();
  const { scenario, scenarioTable } = travel;
  const table = useRows(project, scenarioTable, WHOLE);
  const notices = tableNotices(scenario, table.data);
  const messages: Message[] = [
    ...(notices.unknownMode ? [{ kind: "info" as const, text: f("srv_analysis_access_unmatched_transport_warning", MODES.join(","), "WALKING") }] : []),
    ...(notices.unmatched ? [{ kind: "info" as const, k: "srv_analysis_access_unmatched_scenario_labels_warning" as const }] : []),
  ];
  const selected = useMemo(() => tableRows(table.data), [table.data]);
  return (
    <>
      <CenterTitle title="analysis_travel_scenario_title" sub="analysis_travel_scenario_sub" />
      <div className="px-[15px]">
        <SubTitle k="analysis_travel_scenario_processed" />
        {travel.hasLandCover ? (
          <>
            <LinkActions
              actions={[
                { icon: "reset", k: "analysis_travel_scenario_reset", onClick: () => dispatch({ type: "resetScenario" }) },
                {
                  icon: "edit",
                  k: "analysis_travel_scenario_import_selected_table",
                  onClick: () => dispatch({ type: "scenario", rows: mergeTable(scenario, selected) }),
                },
              ]}
            />
            <DataTable
              columns={SCENARIO_COLUMNS}
              data={scenario as unknown as Row[]}
              height="300px"
              rowId="class"
              onCellEdited={(row, field, value) => {
                if (field === "speed" || field === "mode") dispatch({ type: "scenario", rows: editRow(scenario, Number(row.class), field, value) });
              }}
            />
          </>
        ) : (
          <MessageList messages={[{ kind: "info", k: "analysis_travel_scenario_add_lc" }]} />
        )}
        <SubTitle k="analysis_travel_scenario_selected_table" />
        {scenarioTable === null ? (
          <MessageList messages={[{ kind: "info", k: "analysis_travel_scenario_add_table" }]} />
        ) : (
          <DataTable columns={SCENARIO_COLUMNS.map((c) => ({ field: c.field }))} data={selected as unknown as Row[]} height="300px" />
        )}
        {messages.length > 0 && <MessageList messages={messages} />}
      </div>
    </>
  );
}

