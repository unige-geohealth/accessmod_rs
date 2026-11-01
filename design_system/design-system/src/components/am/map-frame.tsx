import maplibregl, { type StyleSpecification } from "maplibre-gl";
import { Protocol } from "pmtiles";
import "maplibre-gl/dist/maplibre-gl.css";
import "./map-frame.css";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "#ui/components/ui/tooltip";
import { useI18n } from "#ui/i18n";
import type { Key } from "#ui/i18n";
import { cn } from "#ui/lib/utils";
import { type IconName, icons } from "./icons";

let protocol: Protocol | null = null;
function registerPmtiles() {
  if (protocol) return;
  protocol = new Protocol();
  maplibregl.addProtocol("pmtiles", protocol.tile);
}

const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(`--${name}`).trim();

/** Basemap paint, read from the theme tokens (re-applied on theme change). */
const basemapPaint = () =>
  ({
    background: { "background-color": token("map-water") },
    "countries-fill": { "fill-color": token("map-land") },
    "countries-line": { "line-color": token("map-border") },
  }) as const;

/**
 * Offline schematic basemap: one PMTiles archive with a `countries`
 * polygon layer (scripts/build-basemap.sh). No labels, no glyphs, no network.
 */
export function schematicStyle(url = "/basemap/countries.pmtiles"): StyleSpecification {
  const p = basemapPaint();
  return {
    version: 8,
    sources: {
      countries: { type: "vector", url: `pmtiles://${new URL(url, location.href).href}`, attribution: "© Natural Earth" },
    },
    layers: [
      { id: "background", type: "background", paint: p.background },
      { id: "countries-fill", type: "fill", source: "countries", "source-layer": "countries", paint: p["countries-fill"] },
      {
        id: "countries-line",
        type: "line",
        source: "countries",
        "source-layer": "countries",
        paint: { ...p["countries-line"], "line-width": ["interpolate", ["linear"], ["zoom"], 2, 0.5, 8, 1.5] },
      },
    ],
  };
}

export type MapEditing = {
  active: boolean;
  onToggle: () => void;
  /** Jump to the next facility to check (e.g. next one on a barrier). */
  onNext?: () => void;
  onUndo?: () => void;
  onRedo?: () => void;
  canUndo?: boolean;
  canRedo?: boolean;
};

type MapFrameProps = {
  /** MapLibre style; defaults to the offline schematic basemap. */
  mapStyle?: string | StyleSpecification;
  /** Project extent; also the target of the home button. */
  bounds?: [number, number, number, number];
  /** Called once the map is loaded, to add sources and layers. */
  onLoad?: (map: maplibregl.Map) => void;
  /** A style, source or tile failed to load (e.g. an online basemap offline). */
  onError?: (error: Error & { url?: string }) => void;
  /** Facility relocation tools (Toolbox › Interactive map). */
  editing?: MapEditing;
  className?: string;
};

/**
 * Map area of Project summary and Toolbox › Interactive map (MapLibre).
 * Controls are React buttons on top of the map, themed with tokens:
 * zoom and home top-left, facility editing top-right, scale bottom-left.
 */
export function MapFrame({ mapStyle, bounds, onLoad, onError, editing, className }: MapFrameProps) {
  const el = useRef<HTMLDivElement>(null);
  const [map, setMap] = useState<maplibregl.Map | null>(null);
  const onLoadRef = useRef(onLoad);
  onLoadRef.current = onLoad;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    if (!el.current) return;
    registerPmtiles();
    const m = new maplibregl.Map({
      container: el.current,
      style: mapStyle ?? schematicStyle(),
      bounds,
      fitBoundsOptions: { padding: 20 },
      attributionControl: { compact: false },
    });
    m.addControl(new maplibregl.ScaleControl({ unit: "metric" }), "bottom-left");
    m.on("load", () => onLoadRef.current?.(m));
    m.on("error", (e) => onErrorRef.current?.(e.error));

    // follow the theme: repaint basemap layers when <html class> changes
    const repaint = () => {
      if (mapStyle || !m.isStyleLoaded()) return;
      for (const [layer, paint] of Object.entries(basemapPaint())) {
        for (const [prop, value] of Object.entries(paint)) m.setPaintProperty(layer, prop, value);
      }
    };
    const obs = new MutationObserver(repaint);
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });

    setMap(m);
    return () => {
      obs.disconnect();
      m.remove();
      setMap(null);
    };
    // bounds by value: an inline array must not recreate the map
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mapStyle, JSON.stringify(bounds)]);

  return (
    <div className={cn("am-map relative h-[640px] w-full overflow-hidden", className)}>
      <div ref={el} className="size-full" />
      {map && (
        <>
          <ControlGroup className="top-2.5 left-2.5">
            <ControlButton icon="zoomIn" label="map.zoom_in" onClick={() => map.zoomIn()} />
            <ControlButton icon="zoomOut" label="map.zoom_out" onClick={() => map.zoomOut()} />
          </ControlGroup>
          {bounds && (
            <ControlGroup className="top-[92px] left-2.5">
              <ControlButton icon="home" label="map.home" onClick={() => map.fitBounds(bounds, { padding: 20 })} />
            </ControlGroup>
          )}
          {editing && (
            <ControlGroup className="top-2.5 right-2.5">
              <ControlButton icon="edit" label="map.edit" pressed={editing.active} onClick={editing.onToggle} />
              <ControlButton icon="next" label="map.next" disabled={!editing.active || !editing.onNext} onClick={editing.onNext} />
              <ControlButton icon="undo" label="map.undo" disabled={!editing.active || !editing.canUndo} onClick={editing.onUndo} />
              <ControlButton icon="redo" label="map.redo" disabled={!editing.active || !editing.canRedo} onClick={editing.onRedo} />
            </ControlGroup>
          )}
        </>
      )}
    </div>
  );
}

function ControlGroup({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div
      className={cn(
        "absolute z-10 flex flex-col overflow-hidden rounded-md border border-map-control-border bg-map-control shadow-md",
        "divide-y divide-map-control-border",
        className,
      )}
    >
      {children}
    </div>
  );
}

function ControlButton({
  icon,
  label,
  onClick,
  pressed,
  disabled,
}: {
  icon: IconName;
  label: Key;
  onClick?: () => void;
  pressed?: boolean;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const Icon = icons[icon];
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={t(label)}
          aria-pressed={pressed}
          disabled={disabled}
          onClick={onClick}
          className={cn(
            "flex size-8 items-center justify-center text-foreground transition-colors",
            "hover:bg-map-control-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
            "disabled:pointer-events-none disabled:opacity-35",
            pressed && "bg-primary text-primary-foreground hover:bg-primary/90",
          )}
        >
          <Icon className="size-4" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{t(label)}</TooltipContent>
    </Tooltip>
  );
}
