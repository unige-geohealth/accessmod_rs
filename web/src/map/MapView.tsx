// MapLibre adapter: the map shows what its props say and reports clicks.
// It holds no domain state; effects only sync the imperative map from
// props (docs/ui.md, "State").
import maplibregl, { type Map as MlMap } from "maplibre-gl";
import { useEffect, useRef } from "react";
import { api } from "../api/client";
import type { Api } from "../generated/types";

const BLANK: maplibregl.StyleSpecification = {
  version: 8,
  sources: {},
  layers: [{ id: "background", type: "background", paint: { "background-color": "#f4f2ee" } }],
};

const OSM: maplibregl.RasterSourceSpecification = {
  type: "raster",
  tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
  tileSize: 256,
  attribution: "© OpenStreetMap contributors",
};

export function MapView({
  project,
  bounds,
  dataset,
  basemap,
  onClick,
}: {
  project: string;
  bounds: [number, number, number, number] | undefined;
  dataset: Api.Dataset | null;
  basemap: boolean;
  onClick: (lon: number, lat: number) => void;
}) {
  const container = useRef<HTMLDivElement>(null);
  const map = useRef<MlMap | null>(null);
  const click = useRef(onClick);
  click.current = onClick;

  // Create the map once.
  useEffect(() => {
    if (!container.current) return;
    const created = new maplibregl.Map({ container: container.current, style: BLANK });
    created.addControl(new maplibregl.NavigationControl({ showCompass: false }));
    created.addControl(new maplibregl.ScaleControl({ unit: "metric" }));
    created.on("click", (e) => click.current(e.lngLat.lng, e.lngLat.lat));
    map.current = created;
    return () => {
      created.remove();
      map.current = null;
    };
  }, []);

  // Fit the project extent.
  useEffect(() => {
    const m = map.current;
    if (m && bounds) {
      m.fitBounds(bounds, { padding: 20, animate: false });
    }
  }, [bounds]);

  // Basemap.
  useEffect(() => {
    const m = map.current;
    if (!m) return;
    const apply = () => {
      if (basemap && !m.getSource("osm")) {
        m.addSource("osm", OSM);
        m.addLayer(
          { id: "osm", type: "raster", source: "osm" },
          m.getLayer("data") ? "data" : undefined,
        );
      }
      if (!basemap && m.getSource("osm")) {
        m.removeLayer("osm");
        m.removeSource("osm");
      }
    };
    if (m.isStyleLoaded()) apply();
    else m.once("load", apply);
  }, [basemap]);

  // The dataset shown.
  useEffect(() => {
    const m = map.current;
    if (!m) return;
    let cancelled = false;
    const clear = () => {
      for (const id of ["data", "data-line", "data-point"]) {
        if (m.getLayer(id)) m.removeLayer(id);
      }
      if (m.getSource("data")) m.removeSource("data");
    };
    const show = async () => {
      clear();
      if (!dataset) return;
      const kind = dataset.summary.kind;
      if (kind === "raster") {
        m.addSource("data", {
          type: "raster",
          tiles: [api.tileUrl(project, dataset.id)],
          tileSize: 256,
        });
        m.addLayer({
          id: "data",
          type: "raster",
          source: "data",
          paint: { "raster-opacity": 0.85, "raster-resampling": "nearest" },
        });
      } else if (kind === "vector") {
        const features = await api.features(project, dataset.id);
        if (cancelled) return;
        m.addSource("data", { type: "geojson", data: features });
        m.addLayer({
          id: "data",
          type: "fill",
          source: "data",
          filter: ["==", ["geometry-type"], "Polygon"],
          paint: { "fill-color": "#3b7dd8", "fill-opacity": 0.35, "fill-outline-color": "#1d4f91" },
        });
        m.addLayer({
          id: "data-line",
          type: "line",
          source: "data",
          filter: ["==", ["geometry-type"], "LineString"],
          paint: { "line-color": "#c2410c", "line-width": 1.5 },
        });
        m.addLayer({
          id: "data-point",
          type: "circle",
          source: "data",
          filter: ["==", ["geometry-type"], "Point"],
          paint: {
            "circle-radius": 5,
            "circle-color": "#be123c",
            "circle-stroke-color": "#fff",
            "circle-stroke-width": 1.5,
          },
        });
      }
    };
    const run = () => {
      void show();
    };
    if (m.isStyleLoaded()) run();
    else m.once("load", run);
    return () => {
      cancelled = true;
    };
  }, [project, dataset]);

  return <div className="map" ref={container} />;
}
