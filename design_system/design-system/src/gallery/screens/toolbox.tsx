import type maplibregl from "maplibre-gl";
import { useCallback, useState } from "react";
import { Slider } from "@/components/ui/slider";
import { Columns } from "@/components/am/columns";
import { DataTable } from "@/components/am/data-table";
import { CheckboxField, Field, RadioField, SelectField, TextField } from "@/components/am/form";
import { MapFrame } from "@/components/am/map-frame";
import { AmButton, LinkActions, MessageList } from "@/components/am/messages";
import { PageTabs } from "@/components/am/page-tabs";
import { PanelAccordion } from "@/components/am/panel-accordion";
import { CenterTitle, SectionCard, SubTitle } from "@/components/am/section";
import { type SectionTab, SectionTabs } from "@/components/am/section-tabs";
import { type Stack, StackList } from "@/components/am/stack-list";
import { T } from "@/i18n";
import { facilityPoints, landCover, layers, stack as stackMock, zonalLayers } from "../mock";

export type Tool = "map" | "merge" | "pop";
const toolTabs: SectionTab<Tool>[] = [
  { value: "map", label: "tools_map_btn", icon: "map" },
  { value: "merge", label: "tools_merge_lc_btn", icon: "mergeLandCover" },
  { value: "pop", label: "tools_correct_pop_btn", icon: "adjustPopulation" },
];

type MergeTab = "landCover" | "roads" | "barriers" | "merge";

export function ToolboxScreen({ initial = "map", mergeTab }: { initial?: Tool; mergeTab?: MergeTab }) {
  const [tool, setTool] = useState<Tool>(initial);
  return (
    <div className="flex flex-col gap-5">
      <SectionTabs tabs={toolTabs} value={tool} onChange={setTool} />
      {tool === "map" && <InteractiveMap />}
      {tool === "merge" && <MergeLandCover initial={mergeTab} />}
      {tool === "pop" && <AdjustPopulation />}
    </div>
  );
}

const malawi: [number, number, number, number] = [34.4, -16.4, 35.6, -14.6];

function InteractiveMap() {
  const [opacity, setOpacity] = useState([0.8]);
  const [layer, setLayer] = useState(layers.travelTime[0].value);
  const [editing, setEditing] = useState(false);
  const [history, setHistory] = useState({ done: 0, undone: 0 });

  // demo overlay: project extent and facility points (stable for MapFrame)
  const onLoad = useCallback((map: maplibregl.Map) => {
    const primary = getComputedStyle(document.documentElement).getPropertyValue("--primary").trim();
    map.addSource("facilities", {
      type: "geojson",
      data: {
        type: "FeatureCollection",
        features: facilityPoints.map((c) => ({ type: "Feature", properties: {}, geometry: { type: "Point", coordinates: c } })),
      },
    });
    map.addLayer({
      id: "facilities",
      type: "circle",
      source: "facilities",
      paint: { "circle-radius": 6, "circle-color": "#5cb85c", "circle-stroke-color": "#ffffff", "circle-stroke-width": 2 },
    });
    map.addSource("extent", {
      type: "geojson",
      data: {
        type: "Feature",
        properties: {},
        geometry: {
          type: "Polygon",
          coordinates: [[[34.75, -15.85], [35.25, -15.85], [35.25, -15.3], [34.75, -15.3], [34.75, -15.85]]],
        },
      },
    });
    map.addLayer({ id: "extent", type: "line", source: "extent", paint: { "line-color": primary, "line-width": 2 } }, "facilities");
  }, []);

  return (
    <div className="grid grid-cols-12 gap-x-[30px]">
      <div className="col-span-12 md:col-span-4">
        <PanelAccordion
          defaultValue="raster"
          items={[
            {
              value: "options",
              icon: "map",
              title: "tool_map_options",
              content: <SelectField label="toolbox_map_basemap_select" options={[{ value: "schematic", label: "Schematic (offline)" }]} value="schematic" />,
            },
            {
              value: "raster",
              icon: "raster",
              title: "tool_map_raster",
              content: (
                <div className="flex flex-col gap-[15px]">
                  <SelectField label="toolbox_map_raster_select" options={layers.travelTime} value={layer} onChange={setLayer} />
                  <Field label="toolbox_map_opacity">
                    <div className="flex items-center gap-3 pt-2">
                      <Slider min={0} max={1} step={0.1} value={opacity} onValueChange={setOpacity} />
                      <span className="w-8 tabular-nums">{opacity[0]}</span>
                    </div>
                  </Field>
                  <div>
                    <div className="font-bold">
                      <T k="toolbox_map_click_map" />
                    </div>
                    <ul className="list-disc pl-10">
                      {["Easting", "Northing", "Value", "Label"].map((k) => (
                        <li key={k}>
                          <b>{k}</b> : -
                        </li>
                      ))}
                    </ul>
                  </div>
                </div>
              ),
            },
            {
              value: "facilities",
              icon: "coverage",
              title: "tool_map_facilities",
              content: <SelectField label="toolbox_map_facilities_select" options={layers.facilities} value={layers.facilities[0].value} />,
            },
          ]}
        />
      </div>
      <div className="col-span-12 md:col-span-8">
        <MapFrame
          bounds={malawi}
          onLoad={onLoad}
          editing={{
            active: editing,
            onToggle: () => setEditing(!editing),
            onNext: () => setHistory({ done: history.done + 1, undone: 0 }),
            onUndo: () => setHistory({ done: history.done - 1, undone: history.undone + 1 }),
            onRedo: () => setHistory({ done: history.done + 1, undone: history.undone - 1 }),
            canUndo: history.done > 0,
            canRedo: history.undone > 0,
          }}
        />
      </div>
    </div>
  );
}

const labelColumns = [{ field: "class" }, { field: "label", editable: true }];

function MergeLandCover({ initial = "landCover" }: { initial?: MergeTab }) {
  const [stack, setStack] = useState<Stack>(stackMock);
  const [clean, setClean] = useState(true);
  const [barrierType, setBarrierType] = useState("lines");

  const left = (title: Parameters<typeof SectionCard>[0]["title"], sub: Parameters<typeof SectionCard>[0]["sub"], body: React.ReactNode) => (
    <SectionCard title={title} sub={sub}>
      {body}
    </SectionCard>
  );

  return (
    <>
      <CenterTitle title="toolbox_land_cover_merge_main" sub="toolbox_land_cover_merge_main_sub" />
      <PageTabs<MergeTab>
        defaultValue={initial}
        tabs={[
          {
            value: "landCover",
            label: "toolbox_land_cover_tab",
            content: (
              <Columns
                wide
                left={left(
                  "toolbox_land_cover_title",
                  "toolbox_land_cover_add_land_cover_sub",
                  <>
                    <SelectField label="toolbox_land_cover_select_raster" options={landCover.rasters} value={landCover.rasters[0].value} />
                    <SelectField label="toolbox_land_cover_table" options={landCover.tables} value={landCover.tables[0].value} />
                    <MessageList messages={[{ kind: "issue", k: "srv_merge_landcover_table_missing_or_empty" }]} />
                    <AmButton variant="danger" block disabled k="toolbox_land_cover_add_to_stack_btn" />
                  </>,
                )}
                right={
                  <div className="max-w-xl">
                    <SubTitle k="toolbox_land_cover_labeling_table" />
                    <p className="mb-2.5">
                      <T k="toolbox_land_cover_label_sub" />
                    </p>
                    <LinkActions
                      actions={[
                        { icon: "reset", k: "toolbox_land_cover_reset_to_original" },
                        { icon: "edit", k: "toolbox_land_cover_import_labels" },
                        { icon: "help", label: "Help" },
                      ]}
                    />
                    <DataTable columns={labelColumns} data={landCover.rasterLabels} height="270px" />
                    <p className="my-2.5">
                      <T k="toolbox_land_cover_label_optional" />
                    </p>
                    <DataTable columns={labelColumns} data={landCover.tableLabels} height="270px" />
                  </div>
                }
              />
            ),
          },
          {
            value: "roads",
            label: "toolbox_land_cover_roads_tab",
            content: (
              <Columns
                wide
                left={left(
                  "toolbox_land_cover_roads_title",
                  "toolbox_land_cover_roads_sub",
                  <>
                    <SelectField label="toolbox_land_cover_road_layer_vector" options={landCover.roads} value={landCover.roads[0].value} />
                    <SelectField label="toolbox_land_cover_road_column" options={landCover.roadClass} value="class" />
                    <SelectField label="toolbox_land_cover_road_label" options={landCover.roadLabel} value="label" />
                    <AmButton variant="danger" block k="toolbox_land_cover_road_add_btn" />
                  </>,
                )}
                right={
                  <div className="max-w-xl">
                    <SubTitle k="toolbox_land_cover_road_class_label" />
                    <DataTable
                      columns={labelColumns}
                      data={[
                        { class: 1001, label: "Main road" },
                        { class: 1002, label: "Secondary" },
                        { class: 1003, label: "Tertiary" },
                      ]}
                      height="270px"
                    />
                  </div>
                }
              />
            ),
          },
          {
            value: "barriers",
            label: "toolbox_land_cover_barrier_tab",
            content: (
              <Columns
                wide
                left={left(
                  "toolbox_land_cover_barrier_title",
                  "toolbox_land_cover_barrier_sub",
                  <>
                    <p>
                      <T k="toolbox_land_cover_barrier_add_several" />
                    </p>
                    <SelectField label="toolbox_land_cover_barrier_select_vector" options={landCover.barriers} value={landCover.barriers[0].value} />
                    <RadioField
                      label="toolbox_land_cover_barrier_type"
                      value={barrierType}
                      onChange={setBarrierType}
                      options={[
                        { value: "polygons", label: "toolbox_land_cover_barrier_type_polygons" },
                        { value: "lines", label: "toolbox_land_cover_barrier_type_lines" },
                        { value: "points", label: "toolbox_land_cover_barrier_type_points" },
                      ]}
                    />
                    <AmButton variant="danger" block k="toolbox_land_cover_barrier_add_btn" />
                  </>,
                )}
                right={
                  <div className="max-w-xl">
                    <SubTitle k="toolbox_land_cover_barrier_selected_layer" />
                    <DataTable
                      columns={[{ field: "type" }, { field: "count" }]}
                      data={[
                        { type: "lines", count: 412 },
                        { type: "polygons", count: 0 },
                      ]}
                      height="160px"
                    />
                  </div>
                }
              />
            ),
          },
          {
            value: "merge",
            label: "toolbox_land_cover_merge_tab",
            content: (
              <Columns
                wide
                left={left(
                  "toolbox_land_cover_merge_title",
                  "toolbox_land_cover_merge_sub",
                  <>
                    <Field label="toolbox_land_cover_merge_manage">
                      <div className="flex flex-col">
                        <AmButton block k="toolbox_land_cover_merge_skip_btn" onClick={() => setStack({ use: [], skip: [...stack.use, ...stack.skip] })} />
                        <AmButton block k="toolbox_land_cover_merge_all_btn" className="-mt-px" onClick={() => setStack({ use: [...stack.use, ...stack.skip], skip: [] })} />
                        <AmButton block k="toolbox_land_cover_merge_del_btn" className="-mt-px" />
                      </div>
                    </Field>
                    <CheckboxField
                      label="toolbox_land_cover_merge_clean_artefact"
                      help="toolbox_land_cover_merge_clean_artefact_desc"
                      checked={clean}
                      onChange={setClean}
                    />
                    <TextField label="toolbox_land_cover_merge_tag" />
                    <AmButton variant="danger" block disabled={!stack.use.length} k="toolbox_land_cover_merge_btn" />
                  </>,
                )}
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
                            <StackList value={stack} onChange={setStack} />
                          </>
                        ),
                      },
                      { value: "conflicts", title: "toolbox_land_cover_merge_conflict_warning", content: "—" },
                    ]}
                  />
                }
              />
            ),
          },
        ]}
      />
    </>
  );
}

function AdjustPopulation() {
  const [mode, setMode] = useState("notKnown");
  return (
    <>
      <CenterTitle title="toolbox_popcor_main_title" sub="toolbox_popcor_sub" />
      <div className="grid grid-cols-12">
        <div className="col-span-12 md:col-span-4">
          <SectionCard title="toolbox_popcor_config_title" sub="toolbox_popcor_config_sub">
            <SelectField label="toolbox_popcor_select_pop_raster" options={layers.population} value={layers.population[0].value} />
            <SelectField label="toolbox_popcor_select_merged_lc_raster" options={layers.landCoverMerged} value={layers.landCoverMerged[0].value} />
            <hr className="border-separator" />
            <RadioField
              label="toolbox_popcor_mode"
              value={mode}
              onChange={setMode}
              options={[
                { value: "known", label: "toolbox_popcor_mode_pop_known" },
                { value: "notKnown", label: "toolbox_popcor_mode_pop_not_known" },
              ]}
            />
            <SelectField label="toolbox_popcor_select_zones_vector" options={zonalLayers.zones} value={zonalLayers.zones[0].value} />
            <hr className="border-separator" />
            <TextField label="toolbox_popcor_add_tag" />
            <MessageList messages={[{ kind: "issue", k: "srv_pop_correction_missing_tags" }]} />
            <AmButton variant="danger" block disabled k="toolbox_popcor_compute_btn" />
          </SectionCard>
        </div>
      </div>
    </>
  );
}
