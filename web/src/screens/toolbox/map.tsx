// Toolbox › Interactive map (AM5 `amGisPreview`): a raster of the project
// over the offline basemap, its value where the map is clicked, the
// facilities with a label on hover, and facility relocation: edit mode,
// drag, undo and redo, then Save as a new facilities dataset (the source
// layer stays as it is, so runs that read it replay unchanged).
import { Field, TextField } from "@accessmod/ui/am/form";
import { MapFrame } from "@accessmod/ui/am/map-frame";
import { AmButton, MessageList } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { T, useI18n } from "@accessmod/ui/i18n";
import { Slider } from "@accessmod/ui/ui/slider";
import maplibregl, { type GeoJSONSource, type GeoJSONSourceSpecification } from "maplibre-gl";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, issuesOf } from "../../api/client";
import { useBounds, useFeatures, useValue } from "../../api/queries";
import type { Api } from "../../generated/types";
import { datasetLabel, datasetOptions, kindOf, normalizeTags } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import {
  EMPTY,
  type MapDraft,
  type MapMessage,
  type Move,
  moves,
  type PointCollection,
  saveIssues,
  updateMap,
  withMapDefaults,
  withMoves,
} from "../../state/map-tool";
import { useStore } from "../../state/store";
import { useFmt } from "../common";
import { DatasetSelect, FieldSelect, fieldsOf, firstOf, useCatalogue } from "../analysis/inputs";

const RASTER = "am-raster";
const FACILITIES = "am-facilities";

/** GeoJSON data of a MapLibre source. */
type Data = GeoJSONSourceSpecification["data"];

export function InteractiveMap({ project }: { project: string }) {
  const { t } = useI18n();
  const store = draftStore(project, "interactive-map", EMPTY, updateMap);
  const stored = useStore(store, (s) => s);
  const dispatch = store.dispatch;
  const { datasets, byId } = useCatalogue(project);
  const rasters = (datasets ?? [])
    .filter((d) => kindOf(d.class) === "raster")
    .sort((a, b) => b.id - a.id)
    .map((d) => ({ value: String(d.id), label: datasetLabel(t, d) }));
  const facilityOptions = datasetOptions(t, datasets, ["vFacility"]);
  const draft = withMapDefaults(stored, { raster: firstOf(rasters), facilities: firstOf(facilityOptions) }, (id) => fieldsOf(byId, id));
  const bounds = useBounds(project).data;
  const features = useFeatures(project, draft.facilities);
  const value = useValue(project, draft.raster, draft.clicked);
  const moved = useMemo(() => moves(draft), [draft]);
  const [map, setMap] = useState<maplibregl.Map | null>(null);

  useRasterLayer(map, draft.raster === null ? null : api.tileUrl(project, draft.raster), draft.opacity);
  const collection = useMemo(() => (features.data ? withMoves(features.data as PointCollection, moved) : null), [features.data, moved]);
  useFacilitiesLayer(map, collection, draft, dispatch);

  return (
    <div className="grid grid-cols-12 gap-x-[30px] gap-y-5">
      <div className="col-span-12 md:col-span-4">
        <PanelAccordion
          defaultValue="raster"
          items={[
            {
              value: "raster",
              icon: "raster",
              title: "tool_map_raster",
              content: (
                <div className="flex flex-col gap-[15px]">
                  <DatasetSelect label="toolbox_map_raster_select" options={rasters} value={draft.raster} onChange={(id) => dispatch({ type: "raster", id })} />
                  <Field label="toolbox_map_opacity">
                    <div className="flex items-center gap-3 pt-2">
                      <Slider min={0} max={1} step={0.1} value={[draft.opacity]} onValueChange={([v]) => dispatch({ type: "opacity", value: v ?? 0 })} />
                      <span className="w-8 tabular-nums">{draft.opacity.toFixed(1)}</span>
                    </div>
                  </Field>
                  <div>
                    <div className="font-bold">
                      <T k="toolbox_map_click_map" />
                    </div>
                    <ul className="list-disc pl-10">
                      <li>
                        <b>{t("map.longitude")}</b> : {draft.clicked ? draft.clicked.lon.toFixed(6) : "-"}
                      </li>
                      <li>
                        <b>{t("map.latitude")}</b> : {draft.clicked ? draft.clicked.lat.toFixed(6) : "-"}
                      </li>
                      <li>
                        <b>{t("map.value")}</b> : {value.data?.value ?? "-"}
                      </li>
                    </ul>
                  </div>
                </div>
              ),
            },
            {
              value: "facilities",
              icon: "coverage",
              title: "tool_map_facilities",
              content: (
                <FacilitiesPanel
                  project={project}
                  draft={draft}
                  byId={byId}
                  options={facilityOptions}
                  truncated={features.data?.truncated ?? false}
                  dispatch={dispatch}
                />
              ),
            },
          ]}
        />
      </div>
      <div className="col-span-12 md:col-span-8">
        {bounds && (
          <MapFrame
            key={project}
            bounds={bounds}
            onLoad={setMap}
            editing={{
              active: draft.editing,
              onToggle: () => dispatch({ type: "editing", on: !draft.editing }),
              onUndo: () => dispatch({ type: "undo" }),
              onRedo: () => dispatch({ type: "redo" }),
              canUndo: draft.history.length > 0,
              canRedo: draft.undone.length > 0,
            }}
          />
        )}
      </div>
    </div>
  );
}

function FacilitiesPanel({
  project,
  draft,
  byId,
  options,
  truncated,
  dispatch,
}: {
  project: string;
  draft: MapDraft;
  byId: ReadonlyMap<number, Api.Dataset>;
  options: { value: string; label: string }[];
  truncated: boolean;
  dispatch: (message: MapMessage) => void;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const moved = moves(draft);
  const issues = saveIssues(draft);
  const [result, setResult] = useState<{ saved?: Api.Dataset; count?: number; failure: string[] }>({ failure: [] });
  const save = async () => {
    if (draft.facilities === null) return;
    try {
      dispatch({ type: "save" });
      const saved = await api.command("dataset.relocate", { project, id: draft.facilities, moves: moved, tags: normalizeTags(draft.tags) });
      dispatch({ type: "saved", id: saved.id });
      setResult({ saved, count: moved.length, failure: [] });
    } catch (error) {
      dispatch({ type: "saveFailed" });
      setResult({ failure: issuesOf(error, t) });
    }
  };
  return (
    <div className="flex flex-col gap-[15px]">
      <DatasetSelect
        label="toolbox_map_facilities_select"
        options={options}
        value={draft.facilities}
        onChange={(id) => {
          setResult({ failure: [] });
          dispatch({ type: "facilities", id });
        }}
      />
      <FieldSelect
        label="toolbox_map_facilities_select_label"
        fields={fieldsOf(byId, draft.facilities)}
        value={draft.labelField}
        onChange={(field) => dispatch({ type: "labelField", field })}
      />
      {truncated && <MessageList messages={[{ kind: "info", text: f("map.features_truncated", 20000) }]} />}
      <p className="text-sm">
        <T k="map.relocate_hint" />
      </p>
      <p className="text-sm">
        <T k="tool_map_relocate_changes_count" /> {moved.length}
      </p>
      <TextField label="toolbox_map_relocate_tag" value={draft.tags} onChange={(text) => dispatch({ type: "tags", text })} />
      {moved.length > 0 && issues.length > 0 && <MessageList messages={issues.map((k) => ({ kind: "issue" as const, k }))} />}
      {result.failure.length > 0 && <MessageList messages={result.failure.map((text) => ({ kind: "issue" as const, text }))} />}
      {result.saved && (
        <MessageList messages={[{ kind: "info", text: `${f("tool_map_relocate_out_saved", result.count ?? 0)} ${datasetLabel(t, result.saved)}` }]} />
      )}
      <AmButton
        variant="danger"
        block
        disabled={moved.length === 0 || issues.length > 0 || draft.saving}
        k="toolbox_map_relocate_save"
        onClick={() => void save()}
      />
    </div>
  );
}

/** The raster of the tool, below the facilities: tiles of the server, `opacity`. */
function useRasterLayer(map: maplibregl.Map | null, url: string | null, opacity: number) {
  useEffect(() => {
    if (map === null || url === null) return;
    map.addSource(RASTER, { type: "raster", tiles: [`${window.location.origin}${url}`], tileSize: 256 });
    map.addLayer({ id: RASTER, type: "raster", source: RASTER, paint: { "raster-opacity": opacity } }, map.getLayer(FACILITIES) ? FACILITIES : undefined);
    return () => {
      if (!map.getStyle()) return;
      if (map.getLayer(RASTER)) map.removeLayer(RASTER);
      if (map.getSource(RASTER)) map.removeSource(RASTER);
    };
    // Opacity changes the paint property only (below).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, url]);
  useEffect(() => {
    if (map?.getLayer(RASTER)) map.setPaintProperty(RASTER, "raster-opacity", opacity);
  }, [map, opacity]);
}

/**
 * The facilities as circles; a popup with the label on hover; in edit mode
 * a facility is dragged and dropped (one move). Elsewhere a click queries
 * the raster. Handlers read the latest values through a ref.
 */
function useFacilitiesLayer(map: maplibregl.Map | null, collection: PointCollection | null, draft: MapDraft, dispatch: (message: MapMessage) => void) {
  const latest = useRef({ draft, collection, dispatch });
  latest.current = { draft, collection, dispatch };

  useEffect(() => {
    if (map === null) return;
    const empty: PointCollection = { type: "FeatureCollection", features: [] };
    map.addSource(FACILITIES, { type: "geojson", data: empty as Data });
    map.addLayer({
      id: FACILITIES,
      type: "circle",
      source: FACILITIES,
      paint: { "circle-radius": 6, "circle-color": "#5cb85c", "circle-stroke-color": "#ffffff", "circle-stroke-width": 2 },
    });
    const source = () => map.getSource<GeoJSONSource>(FACILITIES);
    const popup = new maplibregl.Popup({ closeButton: false, closeOnClick: false });

    const hover = (e: maplibregl.MapLayerMouseEvent) => {
      const feature = e.features?.[0];
      const field = latest.current.draft.labelField;
      map.getCanvas().style.cursor = latest.current.draft.editing ? "grab" : "pointer";
      if (feature === undefined || field === "") return;
      popup
        .setLngLat(e.lngLat)
        .setText(String(feature.properties?.[field] ?? ""))
        .addTo(map);
    };
    const leave = () => {
      map.getCanvas().style.cursor = "";
      popup.remove();
    };
    const press = (e: maplibregl.MapLayerMouseEvent) => {
      const fid = e.features?.[0]?.id;
      const base = latest.current.collection;
      // The layer of the feature grabbed; the drop is dropped if another is shown by then.
      const layer = latest.current.draft.facilities;
      if (!latest.current.draft.editing || typeof fid !== "number" || base === null || layer === null) return;
      // No pan while dragging.
      e.preventDefault();
      popup.remove();
      map.getCanvas().style.cursor = "grabbing";
      const drag = (m: maplibregl.MapMouseEvent) => {
        source()?.setData(withMoves(base, [{ fid, lon: m.lngLat.lng, lat: m.lngLat.lat }]) as Data);
      };
      map.on("mousemove", drag);
      map.once("mouseup", (m) => {
        map.off("mousemove", drag);
        map.getCanvas().style.cursor = "";
        const move: Move = { fid, lon: m.lngLat.lng, lat: m.lngLat.lat };
        if (latest.current.draft.facilities === layer) latest.current.dispatch({ type: "move", layer, move });
      });
    };
    const click = (e: maplibregl.MapMouseEvent) => {
      if (!latest.current.draft.editing) latest.current.dispatch({ type: "click", lon: e.lngLat.lng, lat: e.lngLat.lat });
    };
    map.on("mousemove", FACILITIES, hover);
    map.on("mouseleave", FACILITIES, leave);
    map.on("mousedown", FACILITIES, press);
    map.on("click", click);
    return () => {
      popup.remove();
      map.off("mousemove", FACILITIES, hover);
      map.off("mouseleave", FACILITIES, leave);
      map.off("mousedown", FACILITIES, press);
      map.off("click", click);
      if (!map.getStyle()) return;
      if (map.getLayer(FACILITIES)) map.removeLayer(FACILITIES);
      if (map.getSource(FACILITIES)) map.removeSource(FACILITIES);
    };
  }, [map]);

  useEffect(() => {
    const empty: PointCollection = { type: "FeatureCollection", features: [] };
    map?.getSource<GeoJSONSource>(FACILITIES)?.setData((collection ?? empty) as Data);
  }, [map, collection]);
}
