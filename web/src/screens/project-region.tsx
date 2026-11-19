// Project › New from a region (Project Creator): a country, units picked on
// the map (or the whole country), a passable distance around them, a cell
// size and the layers to fetch. Create queues a task that downloads the
// sources (kept for later projects) and builds the project; a notification
// says when it is ready (docs/project-creator.md).
import { CheckboxField, SelectField, TextField } from "@accessmod/ui/am/form";
import { MapFrame } from "@accessmod/ui/am/map-frame";
import { AmButton, type Message, MessageList } from "@accessmod/ui/am/messages";
import { useI18n } from "@accessmod/ui/i18n";
import type { FeatureCollection } from "geojson";
import type maplibregl from "maplibre-gl";
import { useEffect, useRef, useState } from "react";
import { api, issuesOf } from "../api/client";
import { useCountries, useUnits } from "../api/queries";
import { BUFFERS_KM, creatorStore, LEVELS, type Layers, missing, params, RESOLUTIONS_M } from "../state/creator";
import { useStore } from "../state/store";
import { useFmt } from "./common";

const LAYERS: {
  layer: keyof Layers;
  label: "creator.layer_land_cover" | "creator.layer_population" | "creator.layer_roads" | "creator.layer_waterways" | "creator.layer_facilities";
}[] = [
  { layer: "land_cover", label: "creator.layer_land_cover" },
  { layer: "population", label: "creator.layer_population" },
  { layer: "roads", label: "creator.layer_roads" },
  { layer: "waterways", label: "creator.layer_waterways" },
  { layer: "osm_facilities", label: "creator.layer_facilities" },
];

/** The form, in the left column. */
export function RegionPanel() {
  const { t } = useI18n();
  const f = useFmt();
  const draft = useStore(creatorStore, (s) => s);
  const dispatch = creatorStore.dispatch;
  const countries = useCountries();
  const units = useUnits(draft.iso3, draft.level);
  const [state, setState] = useState<{ kind: "idle" } | { kind: "sending" } | { kind: "queued"; name: string } | { kind: "failed"; issues: string[] }>({
    kind: "idle",
  });
  const chosen = units.data?.features.filter((u) => draft.units.includes(String(u.properties?.id))) ?? [];
  const ready = params(draft);
  const create = async () => {
    if (ready === null) return;
    setState({ kind: "sending" });
    try {
      await api.command("creator.create", ready);
      setState({ kind: "queued", name: ready.name });
    } catch (error) {
      setState({ kind: "failed", issues: issuesOf(error, t) });
    }
  };
  const messages: Message[] = [
    ...missing(draft).map((k) => ({ kind: "info" as const, k })),
    ...(countries.isError
      ? issuesOf(countries.error, t).map((text) => ({
          kind: "issue" as const,
          text,
        }))
      : []),
    ...(units.isError
      ? issuesOf(units.error, t).map((text) => ({
          kind: "issue" as const,
          text,
        }))
      : []),
    ...(state.kind === "failed" ? state.issues.map((text) => ({ kind: "issue" as const, text })) : []),
  ];
  return (
    <div className="flex flex-col gap-[15px]">
      <TextField label="project_new_name_text" value={draft.name} onChange={(text) => dispatch({ type: "name", text })} />
      <SelectField
        label="creator.country"
        options={(countries.data ?? []).map((c) => ({
          value: c.iso3,
          label: c.name,
        }))}
        value={draft.iso3 ?? undefined}
        onChange={(iso3) => dispatch({ type: "country", iso3 })}
      />
      <SelectField
        label="creator.level"
        options={LEVELS.map((l) => ({
          value: String(l),
          label: l === 0 ? t("creator.level_country") : f("creator.level_n", l),
        }))}
        value={String(draft.level)}
        onChange={(v) => dispatch({ type: "level", level: Number(v) })}
      />
      {draft.level > 0 && (
        <p className="text-sm">
          {chosen.length === 0 ? t("creator.units_hint") : f("creator.units_chosen", chosen.map((u) => String(u.properties?.name)).join(", "))}{" "}
          {chosen.length > 0 && (
            <button type="button" className="text-link underline" onClick={() => dispatch({ type: "clearUnits" })}>
              {t("creator.clear")}
            </button>
          )}
        </p>
      )}
      {units.data?.licence && <p className="text-xs text-muted-foreground">{f("creator.boundaries", units.data.licence)}</p>}
      <SelectField
        label="creator.buffer"
        options={BUFFERS_KM.map((km) => ({
          value: String(km),
          label: `${km} km`,
        }))}
        value={String(draft.buffer_km)}
        onChange={(v) => dispatch({ type: "buffer", km: Number(v) })}
      />
      <SelectField
        label="creator.resolution"
        options={RESOLUTIONS_M.map((m) => ({
          value: String(m),
          label: `${m} m`,
        }))}
        value={String(draft.resolution_m)}
        onChange={(v) => dispatch({ type: "resolution", metres: Number(v) })}
      />
      <div className="flex flex-col gap-1.5">
        <p className="text-sm font-bold">{t("creator.layers")}</p>
        <CheckboxField label="creator.layer_dem" checked disabled />
        {LAYERS.map(({ layer, label }) => (
          <CheckboxField key={layer} label={label} checked={draft.layers[layer]} onChange={(on) => dispatch({ type: "layer", layer, on })} />
        ))}
      </div>
      <CheckboxField
        label="creator.default_scenarios"
        checked={draft.default_scenarios}
        onChange={(on) => dispatch({ type: "defaultScenarios", on })}
      />
      {messages.length > 0 && <MessageList messages={messages} />}
      <AmButton
        variant={ready === null ? "danger" : "primary"}
        block
        k="creator.create"
        disabled={ready === null || state.kind === "sending"}
        onClick={() => void create()}
      />
      {state.kind === "queued" && <p className="text-sm">{f("creator.queued", state.name)}</p>}
    </div>
  );
}

const WORLD: [number, number, number, number] = [-170, -55, 180, 75];

function extent(collection: FeatureCollection): [number, number, number, number] {
  let [w, s, e, n] = [Infinity, Infinity, -Infinity, -Infinity];
  const walk = (c: unknown): void => {
    if (Array.isArray(c) && typeof c[0] === "number" && typeof c[1] === "number") {
      w = Math.min(w, c[0]);
      e = Math.max(e, c[0]);
      s = Math.min(s, c[1]);
      n = Math.max(n, c[1]);
    } else if (Array.isArray(c)) c.forEach(walk);
  };
  for (const feature of collection.features) {
    const geometry = feature.geometry as { coordinates?: unknown } | null;
    walk(geometry?.coordinates);
  }
  return Number.isFinite(w) ? [w, s, e, n] : WORLD;
}

/** The units of the chosen country on a map, in the right column: a click
 * chooses or drops a unit. */
export function RegionMap() {
  const { t } = useI18n();
  const iso3 = useStore(creatorStore, (s) => s.iso3);
  const level = useStore(creatorStore, (s) => s.level);
  const selected = useStore(creatorStore, (s) => s.units);
  const units = useUnits(iso3, level);
  // One map per country, level and units: the map of another key is gone.
  const key = `${iso3}-${level}-${units.data ? "units" : "none"}`;
  const mapRef = useRef<{ key: string; map: maplibregl.Map } | null>(null);
  const filter = (ids: string[]): maplibregl.FilterSpecification => (level === 0 ? ["literal", true] : ["in", ["get", "id"], ["literal", ids]]);
  // The selection drawn on the map follows the store (no React state).
  useEffect(() => {
    const current = mapRef.current;
    if (current?.key === key && current.map.getLayer("units-selected")) current.map.setFilter("units-selected", filter(selected));
  });
  const onLoad = (map: maplibregl.Map) => {
    mapRef.current = { key, map };
    if (!units.data) return;
    const primary = getComputedStyle(document.documentElement).getPropertyValue("--primary").trim();
    const muted = getComputedStyle(document.documentElement).getPropertyValue("--muted-foreground").trim();
    map.addSource("units", { type: "geojson", data: units.data });
    map.addLayer({
      id: "units-fill",
      type: "fill",
      source: "units",
      paint: { "fill-color": primary, "fill-opacity": 0.05 },
    });
    map.addLayer({
      id: "units-selected",
      type: "fill",
      source: "units",
      filter: filter(creatorStore.getState().units),
      paint: { "fill-color": primary, "fill-opacity": 0.35 },
    });
    map.addLayer({
      id: "units-line",
      type: "line",
      source: "units",
      paint: { "line-color": muted, "line-width": 1 },
    });
    if (level > 0) {
      map.on("click", "units-fill", (e) => {
        const id = e.features?.[0]?.properties?.id;
        if (typeof id === "string") creatorStore.dispatch({ type: "toggleUnit", id });
      });
      map.on("mouseenter", "units-fill", () => (map.getCanvas().style.cursor = "pointer"));
      map.on("mouseleave", "units-fill", () => (map.getCanvas().style.cursor = ""));
    }
  };
  const status = iso3 === null ? t("creator.need_country") : units.isLoading ? t("creator.loading_units") : null;
  return (
    <div className="flex flex-col gap-2">
      {status && <p className="text-sm text-muted-foreground">{status}</p>}
      <MapFrame
        key={key}
        className="h-[520px]"
        bounds={units.data ? extent(units.data) : WORLD}
        onLoad={onLoad}
      />
    </div>
  );
}
