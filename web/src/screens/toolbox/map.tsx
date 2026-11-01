// Toolbox › Interactive map (AM5 `amGisPreview`), a small GIS module
// (state/map-tool.ts): the facilities relocator over a reference raster,
// additional vector layers in a z-order, and the basemap. Draw order, top
// first: facilities, additional layers, reference raster, basemap.
// Relocations are saved as a new facilities layer: the source layer stays
// as it is, so runs that read it replay unchanged.
import { Field, SelectField, TextField } from "@accessmod/ui/am/form";
import { MapFrame } from "@accessmod/ui/am/map-frame";
import { AmButton, MessageList } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { StackList } from "@accessmod/ui/am/stack-list";
import { type Key, T, useI18n } from "@accessmod/ui/i18n";
import { Slider } from "@accessmod/ui/ui/slider";
import { useQueries } from "@tanstack/react-query";
import maplibregl, { type ExpressionSpecification, type GeoJSONSource, type GeoJSONSourceSpecification } from "maplibre-gl";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, issuesOf } from "../../api/client";
import { keys, useBasemap, useBounds, useFeatures, useValue } from "../../api/queries";
import type { Api } from "../../generated/types";
import { datasetLabel, datasetOptions, kindOf, normalizeTags } from "../../state/catalogue";
import { draftStore } from "../../state/drafts";
import {
  type Basemap,
  BASEMAPS,
  basemapStyle,
  EMPTY,
  MAX_LAYERS,
  type MapDraft,
  type MapMessage,
  type Move,
  moves,
  nextToInspect,
  type PointCollection,
  placed,
  pointOf,
  saveIssues,
  toInspect,
  updateMap,
  withMapDefaults,
  withMoves,
} from "../../state/map-tool";
import { useStore } from "../../state/store";
import { DatasetSelect, FieldSelect, fieldsOf, firstOf, useCatalogue } from "../analysis/inputs";
import { useFmt } from "../common";

const RASTER = "am-raster";
const FACILITIES = "am-facilities";
const VECTOR = "am-vec-";

/** GeoJSON data of a MapLibre source. */
type Data = GeoJSONSourceSpecification["data"];

/** Colours of the additional layers, by position (data colours, not interface tokens). */
const PALETTE = ["#1f77b4", "#ff7f0e", "#9467bd", "#8c564b", "#e377c2", "#17becf", "#bcbd22", "#7f7f7f", "#2ca02c", "#d62728"];

/** AM5 facility green; red and the selection from the tokens. */
const OK = "#5cb85c";
const token = (name: string, fallback: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;

const BASEMAP_NAMES: Record<Basemap, Key> = {
  schematic: "map.basemap_schematic",
  "backdrop-v4": "map.basemap_backdrop",
  "dataviz-v4": "map.basemap_dataviz",
  "satellite-v4": "map.basemap_satellite",
  "outdoor-v4": "map.basemap_outdoor",
};

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
  const merged = datasetOptions(t, datasets, ["rLandCoverMerged"]);
  const draft = withMapDefaults(stored, { reference: firstOf(merged) ?? firstOf(rasters), facilities: firstOf(facilityOptions) }, (id) => fieldsOf(byId, id));
  const bounds = useBounds(project).data;
  const key = useBasemap().data?.maptiler_key ?? null;
  const style = basemapStyle(draft.basemap, key);

  // Facilities with the reference value at each point, and at moved points.
  const features = useFeatures(project, draft.facilities, draft.reference);
  const value = useValue(project, draft.reference, draft.clicked);
  const moved = useMemo(() => moves(draft), [draft]);
  const movedValues = useMovedValues(project, draft.reference, moved);
  const all = useMemo(() => features.data?.features ?? [], [features.data]);
  const inspect = useMemo(() => toInspect(all, movedValues), [all, movedValues]);
  const collection = useMemo(
    () => (features.data ? withMoves(features.data as PointCollection, moved, inspect, draft.selected) : null),
    [features.data, moved, inspect, draft.selected],
  );

  // The map of the current style: a basemap change makes a new map.
  const [loaded, setLoaded] = useState<{ map: maplibregl.Map; style: string | null } | null>(null);
  const map = loaded?.style === style ? loaded.map : null;
  useRasterLayer(map, draft.reference === null ? null : api.tileUrl(project, draft.reference), draft.opacity);
  const truncated = useVectorLayers(map, project, draft.layers, byId);
  useFacilitiesLayer(map, collection, draft, dispatch);

  const next = () => {
    const fid = nextToInspect(all, inspect, draft.selected);
    dispatch({ type: "select", fid });
    const point = pointOf(collection?.features.find((f) => f.id === fid));
    if (map && point) map.easeTo({ center: point, zoom: Math.max(map.getZoom(), 12), duration: 400 });
  };
  const selected = collection?.features.find((f) => f.id === draft.selected);

  return (
    <div className="grid grid-cols-12 gap-x-[30px] gap-y-5">
      <div className="col-span-12 md:col-span-4">
        <PanelAccordion
          defaultValue="relocator"
          items={[
            {
              value: "relocator",
              icon: "coverage",
              title: "map.relocator",
              content: (
                <Relocator
                  project={project}
                  draft={draft}
                  byId={byId}
                  options={{ facilities: facilityOptions, rasters }}
                  counts={{ inspect: inspect.size, total: all.length, truncated: features.data?.truncated ?? false }}
                  value={value.data?.value}
                  selectedLabel={selected === undefined ? null : String(selected.properties?.[draft.labelField] ?? `#${draft.selected}`)}
                  onNext={next}
                  dispatch={dispatch}
                />
              ),
            },
            {
              value: "layers",
              icon: "raster",
              title: "map.layers",
              content: <Layers draft={draft} datasets={datasets ?? []} truncated={truncated} dispatch={dispatch} />,
            },
            { value: "basemap", icon: "map", title: "map.basemap", content: <BasemapPanel draft={draft} hasKey={key !== null} dispatch={dispatch} /> },
          ]}
        />
      </div>
      <div className="col-span-12 md:col-span-8">
        {bounds && (
          <MapFrame
            key={`${project}/${style ?? "schematic"}`}
            bounds={bounds}
            mapStyle={style ?? undefined}
            onLoad={(m) => {
              setLoaded({ map: m, style });
              // A removed map (basemap change, fallback) is never drawn on again.
              m.on("remove", () => setLoaded((current) => (current?.map === m ? null : current)));
            }}
            onError={(error) => {
              if (style !== null && `${error.url ?? ""} ${error.message}`.includes("api.maptiler.com")) dispatch({ type: "basemapFailed" });
            }}
            editing={{
              active: draft.editing,
              onToggle: () => dispatch({ type: "editing", on: !draft.editing }),
              onNext: inspect.size > 0 ? next : undefined,
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

/** Values of the reference raster at the moved points (`undefined` while loading). */
function useMovedValues(project: string, reference: number | null, moved: readonly Move[]): ReadonlyMap<number, number | null | undefined> {
  const results = useQueries({
    queries: moved.map((m) => ({
      queryKey: keys.value(project, reference ?? 0, m.lon, m.lat),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.value(project, reference ?? 0, m.lon, m.lat, signal),
      enabled: reference !== null,
      staleTime: Infinity,
    })),
  });
  const values = results.map((r) => r.data?.value);
  const signature = JSON.stringify(values);
  // Stable while the values are the same.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  return useMemo(() => new Map(moved.map((m, i) => [m.fid, values[i]])), [moved, signature]);
}

function Relocator({
  project,
  draft,
  byId,
  options,
  counts,
  value,
  selectedLabel,
  onNext,
  dispatch,
}: {
  project: string;
  draft: MapDraft;
  byId: ReadonlyMap<number, Api.Dataset>;
  options: { facilities: { value: string; label: string }[]; rasters: { value: string; label: string }[] };
  counts: { inspect: number; total: number; truncated: boolean };
  value: number | null | undefined;
  selectedLabel: string | null;
  onNext: () => void;
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
        options={options.facilities}
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
      <DatasetSelect label="map.reference_raster" options={options.rasters} value={draft.reference} onChange={(id) => dispatch({ type: "reference", id })} />
      <Field label="toolbox_map_opacity">
        <div className="flex items-center gap-3 pt-2">
          <Slider min={0} max={1} step={0.1} value={[draft.opacity]} onValueChange={([v]) => dispatch({ type: "opacity", value: v ?? 0 })} />
          <span className="w-8 tabular-nums">{draft.opacity.toFixed(1)}</span>
        </div>
      </Field>
      {counts.truncated && <MessageList messages={[{ kind: "info", text: f("map.features_truncated", 20000) }]} />}
      <p className="text-sm" role="status">
        {f("map.to_inspect", counts.inspect, counts.total)}
      </p>
      <AmButton block icon="next" k="map.next_facility" disabled={counts.inspect === 0} onClick={onNext} />
      {selectedLabel !== null && <p className="text-sm">{f("map.selected", selectedLabel)}</p>}
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
            <b>{t("map.value")}</b> : {value ?? "-"}
          </li>
        </ul>
      </div>
      <hr className="border-separator" />
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
        <MessageList messages={[{ kind: "info", text: `${f("tool_map_relocate_out_saved", result.count ?? 0).trimEnd()} ${datasetLabel(t, result.saved)}` }]} />
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

function Layers({
  draft,
  datasets,
  truncated,
  dispatch,
}: {
  draft: MapDraft;
  datasets: readonly Api.Dataset[];
  /** Layers shown in part (too many features or bytes). */
  truncated: readonly number[];
  dispatch: (message: MapMessage) => void;
}) {
  const { t } = useI18n();
  const f = useFmt();
  const vectors = datasets.filter((d) => kindOf(d.class) === "vector").sort((a, b) => b.id - a.id);
  const item = (d: Api.Dataset) => ({ id: String(d.id), label: datasetLabel(t, d), kind: "vector" as const });
  const shown = draft.layers.flatMap((id) => vectors.find((d) => d.id === id) ?? []);
  const available = vectors.filter((d) => !draft.layers.includes(d.id));
  return (
    <div className="flex flex-col gap-[15px]">
      <p className="text-sm">{f("map.layers_sub", MAX_LAYERS)}</p>
      <StackList
        labels={{ use: "map.layers_shown", skip: "map.layers_available" }}
        value={{ use: shown.map(item), skip: available.map(item) }}
        onChange={(stack) => dispatch({ type: "layers", shown: stack.use.map((i) => Number(i.id)) })}
      />
      {truncated.length > 0 && (
        <MessageList
          messages={truncated.flatMap((id) => {
            const d = vectors.find((v) => v.id === id);
            return d ? [{ kind: "info" as const, text: `${datasetLabel(t, d)}: ${f("map.features_shown_in_part")}` }] : [];
          })}
        />
      )}
    </div>
  );
}

function BasemapPanel({ draft, hasKey, dispatch }: { draft: MapDraft; hasKey: boolean; dispatch: (message: MapMessage) => void }) {
  const { t } = useI18n();
  const options = BASEMAPS.filter((b) => hasKey || b === "schematic").map((b) => ({ value: b, label: t(BASEMAP_NAMES[b]) }));
  return (
    <div className="flex flex-col gap-[15px]">
      <SelectField
        label="toolbox_map_basemap_select"
        options={options}
        value={draft.basemap}
        onChange={(v) => dispatch({ type: "basemap", basemap: BASEMAPS.find((b) => b === v) ?? "schematic" })}
      />
      {!hasKey && <MessageList messages={[{ kind: "info", k: "map.basemap_no_key" }]} />}
      {draft.basemapFailed && <MessageList messages={[{ kind: "issue", k: "map.basemap_unavailable" }]} />}
    </div>
  );
}

/** The lowest of our layers matching `test`, to insert below it. */
function lowest(map: maplibregl.Map, test: (id: string) => boolean): string | undefined {
  return map.getStyle()?.layers.find((l) => test(l.id))?.id;
}

/** The reference raster, below the additional layers and the facilities. */
function useRasterLayer(map: maplibregl.Map | null, url: string | null, opacity: number) {
  useEffect(() => {
    if (map === null || url === null) return;
    map.addSource(RASTER, { type: "raster", tiles: [`${window.location.origin}${url}`], tileSize: 256 });
    const before = lowest(map, (id) => id.startsWith(VECTOR) || id === FACILITIES);
    map.addLayer({ id: RASTER, type: "raster", source: RASTER, paint: { "raster-opacity": opacity } }, before);
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

/** Geometry of a vector dataset, for its style. */
function geometryOf(dataset: Api.Dataset | undefined): "polygon" | "point" | "line" {
  const geometry = dataset?.summary.kind === "vector" ? dataset.summary.geometry.toLowerCase() : "";
  return geometry.includes("polygon") ? "polygon" : geometry.includes("point") ? "point" : "line";
}

/** The additional layers in their order (top first), between the reference raster and the facilities; returns those shown in part. */
function useVectorLayers(map: maplibregl.Map | null, project: string, layers: readonly number[], byId: ReadonlyMap<number, Api.Dataset>): number[] {
  const results = useQueries({
    queries: layers.map((id) => ({
      queryKey: keys.features(project, id, null),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.features(project, id, null, signal),
      staleTime: Infinity,
    })),
  });
  const latest = useRef({ data: results.map((r) => r.data), byId });
  latest.current = { data: results.map((r) => r.data), byId };
  // Redrawn when the order changes or a layer arrives.
  const signature = layers.map((id, i) => `${id}:${results[i]?.data ? 1 : 0}`).join(",");

  useEffect(() => {
    if (map === null) return;
    const added: string[] = [];
    // Bottom first, each below the facilities: the last added is on top.
    for (let index = layers.length - 1; index >= 0; index--) {
      const id = layers[index];
      const data = latest.current.data[index];
      if (id === undefined || data === undefined) continue;
      const source = `${VECTOR}${id}`;
      const color = PALETTE[index % PALETTE.length] ?? "#7f7f7f";
      const before = map.getLayer(FACILITIES) ? FACILITIES : undefined;
      map.addSource(source, { type: "geojson", data: data as Data });
      switch (geometryOf(latest.current.byId.get(id))) {
        case "polygon":
          map.addLayer({ id: `${source}-fill`, type: "fill", source, paint: { "fill-color": color, "fill-opacity": 0.25 } }, before);
          map.addLayer({ id: `${source}-line`, type: "line", source, paint: { "line-color": color, "line-width": 1 } }, before);
          added.push(`${source}-fill`, `${source}-line`);
          break;
        case "point":
          map.addLayer({ id: `${source}-circle`, type: "circle", source, paint: { "circle-color": color, "circle-radius": 4 } }, before);
          added.push(`${source}-circle`);
          break;
        case "line":
          map.addLayer({ id: `${source}-line`, type: "line", source, paint: { "line-color": color, "line-width": 2 } }, before);
          added.push(`${source}-line`);
          break;
      }
    }
    return () => {
      if (!map.getStyle()) return;
      for (const id of added) if (map.getLayer(id)) map.removeLayer(id);
      for (const id of layers) if (map.getSource(`${VECTOR}${id}`)) map.removeSource(`${VECTOR}${id}`);
    };
    // The signature covers the order and the data.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, signature]);
  return layers.filter((_, i) => results[i]?.data?.truncated === true);
}

/**
 * The facilities on top: green, or red when to inspect; the selected one
 * larger. A popup shows the label on hover; in edit mode a facility is
 * dragged and dropped (one move). Elsewhere a click queries the raster.
 * Handlers read the latest values through a ref.
 */
function useFacilitiesLayer(map: maplibregl.Map | null, collection: PointCollection | null, draft: MapDraft, dispatch: (message: MapMessage) => void) {
  const latest = useRef({ draft, collection, dispatch });
  latest.current = { draft, collection, dispatch };

  useEffect(() => {
    if (map === null) return;
    const empty: PointCollection = { type: "FeatureCollection", features: [] };
    const selected: ExpressionSpecification = ["boolean", ["get", "selected"], false];
    map.addSource(FACILITIES, { type: "geojson", data: empty as Data });
    map.addLayer({
      id: FACILITIES,
      type: "circle",
      source: FACILITIES,
      paint: {
        "circle-radius": ["case", selected, 9, 6],
        "circle-color": ["case", ["boolean", ["get", "inspect"], false], token("--danger", "#d04736"), OK],
        "circle-stroke-color": ["case", selected, token("--primary", "#32769e"), "#ffffff"],
        "circle-stroke-width": ["case", selected, 3, 2],
      },
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
      // The layer of the feature grabbed; the drop is ignored if another is shown by then.
      const layer = latest.current.draft.facilities;
      if (!latest.current.draft.editing || typeof fid !== "number" || base === null || layer === null) return;
      // No pan while dragging.
      e.preventDefault();
      popup.remove();
      map.getCanvas().style.cursor = "grabbing";
      const drag = (m: maplibregl.MapMouseEvent) => {
        source()?.setData(placed(base, [{ fid, lon: m.lngLat.lng, lat: m.lngLat.lat }]) as Data);
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
