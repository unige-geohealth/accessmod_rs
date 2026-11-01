// Toolbox › Merge land cover (AM5 `amAnalysisMergeLandCover`): land cover,
// roads and barriers added to a stack tab by tab, then ordered and merged.
// The stack is the draft (state/merge.ts): nothing is written before Merge.
import { Columns } from "@accessmod/ui/am/columns";
import { type Column, DataTable, type Row } from "@accessmod/ui/am/data-table";
import { CheckboxField, Field, TextField } from "@accessmod/ui/am/form";
import { AmButton, LinkActions, MessageList } from "@accessmod/ui/am/messages";
import { PageTabs } from "@accessmod/ui/am/page-tabs";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { CenterTitle, SectionCard, SubTitle } from "@accessmod/ui/am/section";
import { type Stack, StackList } from "@accessmod/ui/am/stack-list";
import { type Key, T, useI18n } from "@accessmod/ui/i18n";
import { useMemo } from "react";
import schema from "../../../schemas/jobs/merge-land-cover.json";
import { useRows, useValues } from "../../api/queries";
import { navigate, type Route } from "../../app/route";
import type { Api } from "../../generated/types";
import { datasetOptions, pairedTable } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import {
  ANALYSIS,
  barrierItem,
  buildJob,
  conflicts,
  EMPTY,
  initialLabels,
  landCoverIssues,
  landCoverItem,
  type MergeMessage,
  roadIssues,
  roadNotices,
  roadItems,
  shownLabels,
  type StackItem,
  updateMerge,
  withMergeDefaults,
} from "../../state/merge";
import { labelsOf, landCoverClasses } from "../../state/scenario";
import { useStore } from "../../state/store";
import { DatasetSelect, FieldSelect, fieldsOf, firstOf, useCatalogue } from "../analysis/inputs";
import { WHOLE } from "../analysis/travel";
import { ValidationCard } from "../analysis/validation-card";

type Tab = "landCover" | "roads" | "barriers" | "merge";
const TABS: readonly Tab[] = ["landCover", "roads", "barriers", "merge"];
const isTab = (v: string | undefined): v is Tab => TABS.includes(v as Tab);

/** Classes of the stack items (AM5 data classes). Labels of stack items use the AM5 stack classes (`class.rStackRoad`, …). */
const STACK = (schema.$defs.StackItem.oneOf as { properties: { path: { "x-accessmod-class": Api.DataClass[] } } }[]).map(
  (item) => item.properties.path["x-accessmod-class"],
);
const [LAND_COVER = [], ROADS = [], BARRIERS = []] = STACK;

const LABEL_COLUMNS: Column[] = [{ field: "class" }, { field: "label", editable: true }];

export function MergeLandCover({ project, route }: { project: string; route: Route }) {
  const { t } = useI18n();
  const store = draftStore(project, ANALYSIS, EMPTY, updateMerge);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId, dem } = useCatalogue(project);
  const options = {
    landCover: datasetOptions(t, datasets, LAND_COVER),
    labelTable: datasetOptions(t, datasets, ["tLandCover"]),
    roads: datasetOptions(t, datasets, ROADS),
    barrier: datasetOptions(t, datasets, BARRIERS),
  };
  const landCoverDataset = stored.landCover === null ? undefined : byId.get(stored.landCover);
  const paired = pairedTable(datasets, landCoverDataset ?? byId.get(firstOf(options.landCover) ?? -1), "tLandCover")?.id ?? null;
  const draft = withMergeDefaults(
    stored,
    {
      landCover: firstOf(options.landCover),
      labelTable: paired ?? firstOf(options.labelTable),
      roads: firstOf(options.roads),
      barrier: firstOf(options.barrier),
    },
    (id) => fieldsOf(byId, id),
  );
  const tab = isTab(route.query.tab) ? route.query.tab : "landCover";

  const tabs: { value: Tab; label: Key; content: React.ReactNode }[] = [
    {
      value: "landCover",
      label: "toolbox_land_cover_tab",
      content: <LandCoverTab project={project} draft={draft} options={options} byId={byId} dispatch={dispatch} />,
    },
    {
      value: "roads",
      label: "toolbox_land_cover_roads_tab",
      content: <RoadsTab project={project} draft={draft} options={options} byId={byId} dispatch={dispatch} />,
    },
    {
      value: "barriers",
      label: "toolbox_land_cover_barrier_tab",
      content: <BarriersTab draft={draft} options={options} byId={byId} dispatch={dispatch} />,
    },
    {
      value: "merge",
      label: "toolbox_land_cover_merge_tab",
      content: <MergeTab project={project} draft={draft} byId={byId} dem={dem} dispatch={dispatch} />,
    },
  ];
  return (
    <>
      <CenterTitle title="toolbox_land_cover_merge_main" sub="toolbox_land_cover_merge_main_sub" />
      <PageTabs<Tab> tabs={tabs} value={tab} onValueChange={(v) => navigate({ ...route, query: { ...route.query, tab: v } })} />
    </>
  );
}

type Draft = ReturnType<typeof withMergeDefaults>;
type Options = Record<"landCover" | "labelTable" | "roads" | "barrier", { value: string; label: string }[]>;
interface TabProps {
  draft: Draft;
  options: Options;
  byId: ReadonlyMap<number, Api.Dataset>;
  dispatch: (message: MergeMessage) => void;
}

function LandCoverTab({ project, draft, options, byId, dispatch }: TabProps & { project: string }) {
  const landCover = draft.landCover === null ? undefined : byId.get(draft.landCover);
  const table = useRows(project, draft.labelTable, WHOLE);
  const current = table.isPlaceholderData ? undefined : table.data;
  const initial = useMemo(() => initialLabels(landCoverClasses(landCover), labelsOf(current)), [landCover, current]);
  const labels = shownLabels(draft, initial);
  const issues = landCoverIssues(draft.landCover, labels);
  return (
    <Columns
      wide
      left={
        <SectionCard title="toolbox_land_cover_title" sub="toolbox_land_cover_add_land_cover_sub">
          <DatasetSelect
            label="toolbox_land_cover_select_raster"
            options={options.landCover}
            value={draft.landCover}
            onChange={(id) => dispatch({ type: "landCover", id })}
          />
          <DatasetSelect
            label="toolbox_land_cover_table"
            options={options.labelTable}
            value={draft.labelTable}
            onChange={(id) => dispatch({ type: "labelTable", id })}
          />
          {issues.length > 0 && <MessageList messages={issues.map((k) => ({ kind: "issue" as const, k }))} />}
          <AmButton
            variant="danger"
            block
            disabled={issues.length > 0 || draft.landCover === null}
            k="toolbox_land_cover_add_to_stack_btn"
            onClick={() => draft.landCover !== null && dispatch({ type: "add", items: [landCoverItem(draft.landCover, labels)] })}
          />
        </SectionCard>
      }
      right={
        <div className="max-w-xl">
          <SubTitle k="toolbox_land_cover_labeling_table" />
          <p className="mb-2.5">
            <T k="toolbox_land_cover_label_sub" />
          </p>
          <LinkActions actions={[{ icon: "reset", k: "toolbox_land_cover_reset_to_original", onClick: () => dispatch({ type: "resetLabels" }) }]} />
          <DataTable
            columns={LABEL_COLUMNS}
            data={labels as unknown as Row[]}
            rowId="class"
            height="300px"
            onCellEdited={(row, field, value) => {
              if (field !== "label") return;
              dispatch({ type: "labels", labels: labels.map((l) => (l.class === Number(row.class) ? { ...l, label: String(value ?? "") } : l)) });
            }}
          />
        </div>
      }
    />
  );
}

function RoadsTab({ project, draft, options, byId, dispatch }: TabProps & { project: string }) {
  const fields = fieldsOf(byId, draft.roads);
  const values = useValues(project, draft.roads, draft.classField, draft.labelField);
  const classes = values.isFetching ? undefined : values.data;
  const issues = [...roadIssues(draft.roads, classes), ...(values.isError ? (["srv_merge_landcover_road_no_integer_column"] as const) : [])];
  return (
    <Columns
      wide
      left={
        <SectionCard title="toolbox_land_cover_roads_title" sub="toolbox_land_cover_roads_sub">
          <DatasetSelect
            label="toolbox_land_cover_road_layer_vector"
            options={options.roads}
            value={draft.roads}
            onChange={(id) => dispatch({ type: "roads", id })}
          />
          <FieldSelect
            label="toolbox_land_cover_road_column"
            fields={fields}
            value={draft.classField}
            onChange={(field) => dispatch({ type: "classField", field })}
          />
          <FieldSelect
            label="toolbox_land_cover_road_label"
            fields={fields}
            value={draft.labelField}
            onChange={(field) => dispatch({ type: "labelField", field })}
          />
          {issues.length + roadNotices(classes).length > 0 && (
            <MessageList
              messages={[...issues.map((k) => ({ kind: "issue" as const, k })), ...roadNotices(classes).map((k) => ({ kind: "info" as const, k }))]}
            />
          )}
          <AmButton
            variant="danger"
            block
            disabled={issues.length > 0 || classes === undefined || draft.roads === null}
            k="toolbox_land_cover_road_add_btn"
            onClick={() =>
              classes && draft.roads !== null && dispatch({ type: "add", items: roadItems(draft.roads, draft.classField, draft.labelField, classes) })
            }
          />
        </SectionCard>
      }
      right={
        <div className="max-w-xl">
          <SubTitle k="toolbox_land_cover_road_class_label" />
          <DataTable
            columns={[{ field: "class" }, { field: "label" }, { field: "count" }]}
            data={(classes?.values ?? []).map((v) => ({ class: Number(v.value), label: v.label ?? null, count: v.count }))}
            height="300px"
          />
        </div>
      }
    />
  );
}

function BarriersTab({ draft, options, byId, dispatch }: TabProps) {
  const barrier = draft.barrier === null ? undefined : byId.get(draft.barrier);
  const summary = barrier?.summary.kind === "vector" ? barrier.summary : undefined;
  return (
    <Columns
      wide
      left={
        <SectionCard title="toolbox_land_cover_barrier_title" sub="toolbox_land_cover_barrier_sub">
          <p>
            <T k="toolbox_land_cover_barrier_add_several" />
          </p>
          <DatasetSelect
            label="toolbox_land_cover_barrier_select_vector"
            options={options.barrier}
            value={draft.barrier}
            onChange={(id) => dispatch({ type: "barrier", id })}
          />
          {draft.barrier === null && <MessageList messages={[{ kind: "issue", k: "srv_merge_landcover_err_missing_barrier" }]} />}
          <AmButton
            variant="danger"
            block
            disabled={draft.barrier === null}
            k="toolbox_land_cover_barrier_add_btn"
            onClick={() => draft.barrier !== null && dispatch({ type: "add", items: [barrierItem(draft.barrier)] })}
          />
        </SectionCard>
      }
      right={
        <div className="max-w-xl">
          <SubTitle k="toolbox_land_cover_barrier_selected_layer" />
          <DataTable
            columns={[{ field: "type" }, { field: "count" }]}
            data={summary ? [{ type: summary.geometry, count: summary.features }] : []}
            height="120px"
          />
        </div>
      }
    />
  );
}

function MergeTab({
  project,
  draft,
  byId,
  dem,
  dispatch,
}: {
  project: string;
  draft: Draft;
  byId: ReadonlyMap<number, Api.Dataset>;
  dem: number | null;
  dispatch: (message: MergeMessage) => void;
}) {
  const { t } = useI18n();
  const label = (item: StackItem): string => {
    const tags = byId.get(item.dataset)?.tags.join(" ") ?? `#${item.dataset}`;
    switch (item.type) {
      case "land_cover":
        return `${t("class.rStackLandCover" as Key)} [${tags}]`;
      case "road":
        return `${t("class.rStackRoad" as Key)} [${item.label || item.class} ${tags}]`;
      case "barrier":
        return `${t("class.rStackBarrier" as Key)} [${tags}]`;
    }
  };
  const toStack = (items: readonly StackItem[]) =>
    items.map((i) => ({ id: i.key, label: label(i), kind: i.type === "land_cover" ? ("raster" as const) : ("vector" as const) }));
  const all = new Map([...draft.use, ...draft.skip].map((i) => [i.key, i]));
  const fromStack = (s: Stack) => {
    const back = (list: Stack["use"]) => list.flatMap((i) => all.get(i.id) ?? []);
    dispatch({ type: "order", use: back(s.use), skip: back(s.skip) });
  };
  const built = useMemo(() => buildJob(draft, { datasets: byId, dem }), [draft, byId, dem]);
  const found = conflicts(draft.use);
  return (
    <Columns
      wide
      left={
        <>
          <SectionCard title="toolbox_land_cover_merge_title" sub="toolbox_land_cover_merge_sub">
            <Field label="toolbox_land_cover_merge_manage">
              <div className="flex flex-col">
                <AmButton block k="toolbox_land_cover_merge_skip_btn" onClick={() => dispatch({ type: "skipAll" })} />
                <AmButton block k="toolbox_land_cover_merge_all_btn" className="-mt-px" onClick={() => dispatch({ type: "useAll" })} />
                <AmButton block k="toolbox_land_cover_merge_del_btn" className="-mt-px" onClick={() => dispatch({ type: "deleteSkipped" })} />
              </div>
            </Field>
            <CheckboxField
              label="toolbox_land_cover_merge_clean_artefact"
              help="toolbox_land_cover_merge_clean_artefact_desc"
              checked={draft.clean}
              onChange={(on) => dispatch({ type: "clean", on })}
            />
            <TextField label="toolbox_land_cover_merge_tag" value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
          </SectionCard>
          <ValidationCard
            project={project}
            analysis={ANALYSIS}
            issues={built.issues}
            notices={[]}
            document={built.document}
            tags={built.tags}
            datasets={byId}
            button="toolbox_land_cover_merge_btn"
          />
        </>
      }
      right={
        <PanelAccordion
          defaultValue="order"
          items={[
            {
              value: "order",
              title: "toolbox_land_cover_merge_order",
              content: (
                <>
                  <p className="mb-4">
                    <T k="toolbox_land_cover_merge_reorder_mouse" />
                  </p>
                  <StackList value={{ use: toStack(draft.use), skip: toStack(draft.skip) }} onChange={fromStack} />
                </>
              ),
            },
            {
              value: "conflicts",
              title: "toolbox_land_cover_merge_conflict_warning",
              content: (
                <>
                  <p className="mb-2.5">
                    <T k="toolbox_land_cover_merge_conflict_explanation" />
                  </p>
                  <DataTable
                    columns={[{ field: "map" }, { field: "class" }, { field: "label" }]}
                    data={found.map((c) => ({ map: label(c.item), class: c.class, label: c.label }))}
                    height="200px"
                  />
                </>
              ),
            },
          ]}
        />
      }
    />
  );
}
