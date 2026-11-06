// Project (AM5 `amManageProject`): open, new (from a DEM or a project
// file), delete and export on the left; the summary of the open project on
// the right. The language is in Settings.
import { Columns } from "@accessmod/ui/am/columns";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { CheckboxField, SelectField, TextField } from "@accessmod/ui/am/form";
import { MapFrame } from "@accessmod/ui/am/map-frame";
import { AmButton, Callout, FilePicker, LinkActions, MessageList } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { CenterTitle } from "@accessmod/ui/am/section";
import { T, useI18n } from "@accessmod/ui/i18n";
import type maplibregl from "maplibre-gl";
import { useState } from "react";
import { api, issuesOf } from "../api/client";
import { keys, queryClient, useBounds, useProject, useProjects, useSettings } from "../api/queries";
import { HOME, navigate, type Route } from "../app/route";
import { useBatch } from "../state/batch";
import { ACCEPT } from "../state/catalogue";
import { extentDegrees, extentMetric, gridParameters, validName } from "../state/project-summary";
import { demoSlug } from "../state/start";
import { useTransfer } from "../state/transfer";
import { BatchStatus, EntryList, TransferStatus, useFmt } from "./common";

export function ProjectScreen({ route }: { route: Route }) {
  return (
    <Columns
      wide
      left={
        <PanelAccordion
          value={route.panel ?? ""}
          onValueChange={(v) => navigate({ ...route, panel: v === "" ? null : v, query: {} })}
          items={[
            { value: "open", icon: "open", title: "project_open_title", content: <OpenPanel route={route} /> },
            { value: "new", icon: "add", title: "project_new_import_title", content: <NewPanel /> },
            { value: "delete", icon: "delete", title: "project_delete_title", content: <DeletePanel route={route} /> },
            { value: "export", icon: "export", title: "project_export_title", content: <ExportPanel route={route} /> },
          ]}
        />
      }
      right={
        <>
          <CenterTitle title="project_summary_title" sub="project_summary_sub" />
          {route.project === null ? (
            <>
              <Callout k="project.open_first" />
              <DemoHint />
            </>
          ) : (
            <Summary slug={route.project} />
          )}
        </>
      }
    />
  );
}

function projectOptions(projects: { slug: string; info: { name: string } }[] | undefined) {
  return (projects ?? []).map((p) => ({ value: p.slug, label: p.info.name }));
}

/** An invitation to build the demo project again, when it is missing. */
function DemoHint() {
  const projects = useProjects();
  if (!projects.data || demoSlug(projects.data) !== null) return null;
  return (
    <>
      <Callout k="project.demo_missing" className="mb-2.5" />
      <LinkActions actions={[{ k: "toolbox.demo_title", icon: "reset", onClick: () => navigate({ ...HOME, section: "toolbox", panel: "demo" }) }]} />
    </>
  );
}

function OpenPanel({ route }: { route: Route }) {
  const f = useFmt();
  const projects = useProjects();
  const settings = useSettings();
  const { batch, start } = useBatch();
  const preferred = settings.data?.default_project ?? null;
  const setDefault = (project: string | null) =>
    start([project], () => projects.data?.find((p) => p.slug === route.project)?.info.name ?? "", async (p) => {
      await api.command("settings.default_project", { project: p });
      await queryClient.invalidateQueries({ queryKey: keys.settings });
    });
  return (
    <div className="flex flex-col gap-[15px]">
      <SelectField
        label="project_open_select"
        options={projectOptions(projects.data)}
        value={route.project ?? undefined}
        onChange={(slug) => navigate({ ...HOME, project: slug, panel: "open" })}
      />
      {route.project !== null &&
        (preferred === route.project ? (
          <AmButton block icon="close" k="project.default_clear" disabled={batch.state === "running"} onClick={() => void setDefault(null)} />
        ) : (
          <AmButton block icon="home" k="project.default_set" disabled={batch.state === "running"} onClick={() => void setDefault(route.project)} />
        ))}
      {preferred !== null && (
        <p className="text-xs text-muted-foreground">{f("project.default_current", projects.data?.find((p) => p.slug === preferred)?.info.name ?? preferred)}</p>
      )}
      <BatchStatus batch={batch} doneKey="project.default_done" />
    </div>
  );
}

/** A project file (`.am6p`), rather than a DEM. */
const isProjectFile = (files: readonly File[]) => files.length === 1 && files[0]!.name.toLowerCase().endsWith(".am6p");

function NewPanel() {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const { state, start } = useTransfer();
  const busy = state.state === "uploading" || state.state === "processing";
  // AM5 "Choose DEM or project": a DEM creates a project, a project file imports one.
  const create = (files: File[]) =>
    start(files, async (upload) => {
      const source = { upload: { id: upload.id } };
      const created = isProjectFile(files)
        ? await api.command("project.import", { name: name.trim(), package: source })
        : await api.command("project.create", { name: name.trim(), dem: source });
      await queryClient.invalidateQueries({ queryKey: keys.projects });
      navigate({ ...HOME, project: created.slug });
      return created.info.name;
    });
  return (
    <div className="flex flex-col gap-[15px]">
      <TextField label="project_new_name_text" value={name} onChange={setName} />
      {!validName(name) && <p className="text-xs text-muted-foreground">{t("project_new_name_unlock_dem")}</p>}
      <FilePicker block multiple k="project_import_btn" accept={`${ACCEPT.raster},.am6p`} disabled={!validName(name) || busy} onFiles={create} />
      <TransferStatus transfer={state} doneKey="data.imported" />
    </div>
  );
}

function DeletePanel({ route }: { route: Route }) {
  const f = useFmt();
  const projects = useProjects();
  const [chosen, setChosen] = useState<string | undefined>(undefined);
  const [confirming, setConfirming] = useState(false);
  const { batch, start } = useBatch();
  const name = projects.data?.find((p) => p.slug === chosen)?.info.name ?? chosen ?? "";
  const remove = async () => {
    if (chosen === undefined) return;
    const removed = await start([chosen], () => name, (project) => api.command("project.remove", { project }));
    await queryClient.invalidateQueries({ queryKey: keys.projects });
    if (removed.length > 0) {
      setChosen(undefined);
      if (chosen === route.project) navigate({ ...HOME, panel: "delete" });
    }
  };
  return (
    <div className="flex flex-col gap-[15px]">
      <SelectField label="project_delete_select_project" options={projectOptions(projects.data)} value={chosen} onChange={setChosen} />
      <AmButton variant="danger" block k="project_delete_btn" disabled={chosen === undefined || batch.state === "running"} onClick={() => setConfirming(true)} />
      <BatchStatus batch={batch} doneKey="data.removed" />
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} title="srv_project_confirmation_title" confirm="srv_project_delete_confirm_btn" onConfirm={() => void remove()}>
        {f("srv_project_delete_project_confirm", name)}
      </ConfirmDialog>
    </div>
  );
}

/** AM5 Project › Export: the open project as a project file (`.am6p`), downloaded. */
function ExportPanel({ route }: { route: Route }) {
  const { t } = useI18n();
  const [results, setResults] = useState(true);
  const [busy, setBusy] = useState(false);
  const [issues, setIssues] = useState<string[]>([]);
  const slug = route.project;
  const exportProject = async () => {
    if (slug === null) return;
    setBusy(true);
    setIssues([]);
    try {
      const file = await api.command("project.export", { project: slug, results });
      window.location.href = api.exportUrl(slug, file.name);
    } catch (error) {
      setIssues(issuesOf(error, t));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-col gap-[15px]">
      {slug === null && <Callout k="project.open_first" />}
      <CheckboxField label="project.export_results" checked={results} onChange={setResults} />
      <AmButton block k="project_export_btn" disabled={slug === null || busy} onClick={() => void exportProject()} />
      {busy && (
        <p className="text-xs text-muted-foreground" role="status">
          <T k="project_export_progress_text" />
        </p>
      )}
      {issues.length > 0 && <MessageList messages={issues.map((text) => ({ kind: "issue" as const, text }))} />}
    </div>
  );
}

function Summary({ slug }: { slug: string }) {
  const f = useFmt();
  const info = useProject(slug);
  const bounds = useBounds(slug);
  if (info.isError) return <Callout>{f("srv_project_project_not_available", slug)}</Callout>;
  const grid = info.data?.grid;
  return (
    <PanelAccordion
      defaultValue="map"
      items={[
        {
          value: "map",
          title: "project_location_map_title",
          content: bounds.data ? <MapFrame key={slug} className="h-[400px]" bounds={bounds.data} onLoad={(map) => addExtent(map, bounds.data)} /> : null,
        },
        {
          value: "proj",
          title: "project_projection_system_title",
          content: grid ? <pre className="border border-well-border bg-well p-[15px] text-xs whitespace-pre-wrap break-all">{grid.crs_wkt}</pre> : null,
        },
        { value: "grid", title: "project_grid_parameter_title", content: grid ? <EntryList entries={gridParameters(grid)} /> : null },
        { value: "extm", title: "project_extent_metric_title", content: grid ? <EntryList entries={extentMetric(grid)} /> : null },
        { value: "extd", title: "project_extent_degrees_title", content: bounds.data ? <EntryList entries={extentDegrees(bounds.data)} /> : null },
      ]}
    />
  );
}

/** AM5 symbology: the project extent as a primary-coloured outline. */
function addExtent(map: maplibregl.Map, [w, s, e, n]: readonly [number, number, number, number]) {
  const primary = getComputedStyle(document.documentElement).getPropertyValue("--primary").trim();
  map.addSource("extent", {
    type: "geojson",
    data: {
      type: "Feature",
      properties: {},
      geometry: { type: "Polygon", coordinates: [[[w, s], [e, s], [e, n], [w, n], [w, s]]] },
    },
  });
  map.addLayer({ id: "extent", type: "line", source: "extent", paint: { "line-color": primary, "line-width": 2 } });
}
