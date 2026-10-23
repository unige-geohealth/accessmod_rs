import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api/client";
import { useBounds, useClasses, useDatasets } from "../api/queries";
import { className, t } from "../app/i18n";
import { navigate } from "../app/route";
import type { Api } from "../generated/types";
import { MapView } from "../map/MapView";
import { bytes, Issues, Loading } from "./common";

function summary(d: Api.Dataset): string {
  switch (d.summary.kind) {
    case "raster": {
      const s = d.summary.stats;
      if (s.values) return `${s.values.length} classes`;
      return s.min === null || s.min === undefined ? "empty" : `${fmt(s.min)} – ${fmt(s.max ?? 0)}`;
    }
    case "vector":
      return `${d.summary.features} features`;
    case "table":
      return `${d.summary.rows} rows`;
  }
}

const fmt = (v: number) => (Math.abs(v) >= 1000 ? v.toFixed(0) : String(Number(v.toPrecision(4))));

export function DataScreen({ project, selected }: { project: string; selected: string | null }) {
  const datasets = useDatasets(project);
  const bounds = useBounds(project);
  const [basemap, setBasemap] = useState(false);
  // A value read on the map, with the dataset it belongs to: shown only
  // while that dataset is selected (a late answer cannot mislabel another).
  const [probe, setProbe] = useState<{ dataset: number; text: string } | null>(null);
  const current = datasets.data?.find((d) => String(d.id) === selected) ?? null;
  const vector = current?.summary.kind === "vector" ? current : null;
  const features = useQuery({
    queryKey: ["features", project, vector?.id],
    queryFn: ({ signal }) => api.features(project, vector?.id ?? 0, signal),
    enabled: vector !== null,
    staleTime: 60_000,
  });
  const select = (id: number) => navigate({ project, section: "data", item: String(id) });
  const onMapClick = (lon: number, lat: number) => {
    if (current?.summary.kind !== "raster") return;
    const dataset = current.id;
    void api.value(project, dataset, lon, lat).then(
      (r) => setProbe({ dataset, text: r.value === null ? "—" : String(r.value) }),
      () => setProbe(null),
    );
  };
  const probeText = probe !== null && probe.dataset === current?.id ? probe.text : null;
  return (
    <section className="split">
      <div className="left">
        <h1>{t("section.data")}</h1>
        {datasets.isPending && <Loading />}
        <Issues error={datasets.error} />
        {datasets.data?.length === 0 && <p className="muted">{t("data.empty")}</p>}
        <table className="table">
          <thead>
            <tr>
              <th>#</th>
              <th>{t("data.class")}</th>
              <th>{t("data.tags")}</th>
              <th />
              <th />
            </tr>
          </thead>
          <tbody>
            {datasets.data?.map((d) => (
              <tr
                key={d.id}
                className={d === current ? "selected" : ""}
                tabIndex={0}
                aria-selected={d === current}
                onClick={() => select(d.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") select(d.id);
                }}
              >
                <td>{d.id}</td>
                <td title={d.class}>{className(d.class)}</td>
                <td>{d.tags.join(" ")}</td>
                <td className="muted small">{summary(d)}</td>
                <td className="muted small">{bytes(d.bytes)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <Import project={project} />
      </div>
      <div className="right">
        <div className="map-bar">
          <span>
            {current
              ? `${className(current.class)} · ${current.tags.join(" ")}`
              : t("data.preview")}
          </span>
          {features.data?.truncated && (
            <span className="warning">
              {features.data.features.length} /{" "}
              {vector?.summary.kind === "vector" ? vector.summary.features : "?"}
            </span>
          )}
          {probeText !== null && (
            <span className="probe">
              {t("map.value")}: {probeText}
            </span>
          )}
          <label className="inline">
            <input
              type="checkbox"
              checked={basemap}
              onChange={(e) => setBasemap(e.target.checked)}
            />
            {t("map.basemap")}
          </label>
        </div>
        <MapView
          project={project}
          bounds={bounds.data}
          dataset={current}
          features={vector ? (features.data ?? null) : null}
          basemap={basemap}
          onClick={onMapClick}
        />
      </div>
    </section>
  );
}

function Import({ project }: { project: string }) {
  const classes = useClasses();
  const [path, setPath] = useState("");
  const [cls, setCls] = useState("rLandCover");
  const [tags, setTags] = useState("");
  const [layer, setLayer] = useState("");
  const run = useMutation({
    mutationFn: () =>
      api.command<Api.Dataset>("dataset.import", {
        project,
        path,
        class: cls,
        tags: tags.split(/[\s,;]+/).filter((x) => x.length > 0),
        ...(layer ? { layer } : {}),
      }),
    onSuccess: (dataset) => {
      setPath("");
      setTags("");
      navigate({ project, section: "data", item: String(dataset.id) });
    },
  });
  return (
    <form
      className="panel"
      onSubmit={(e) => {
        e.preventDefault();
        run.mutate();
      }}
    >
      <h2>{t("data.import")}</h2>
      <label>
        {t("data.file")}
        <input value={path} onChange={(e) => setPath(e.target.value)} required />
      </label>
      <label>
        {t("data.class")}
        <select value={cls} onChange={(e) => setCls(e.target.value)}>
          {classes.data
            ?.filter((c) => c.importable && c.class !== "rDem")
            .map((c) => (
              <option key={c.class} value={c.class}>
                {className(c.class)} ({c.class})
              </option>
            ))}
        </select>
      </label>
      <label>
        {t("data.tags")}
        <input value={tags} onChange={(e) => setTags(e.target.value)} required />
      </label>
      <label>
        {t("data.layer")}
        <input value={layer} onChange={(e) => setLayer(e.target.value)} />
      </label>
      <button type="submit" disabled={run.isPending}>
        {run.isPending ? t("common.loading") : t("data.import")}
      </button>
      <Issues error={run.error} />
    </form>
  );
}
