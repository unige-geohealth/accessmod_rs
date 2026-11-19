// Project (AM5 `amManageProject`): open, new (from a DEM or a project
// file), delete and export on the left; the summary of the open project on
// the right. The language is in Settings.
import { Columns } from "@accessmod/ui/am/columns";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { CheckboxField, SelectField, TextField } from "@accessmod/ui/am/form";
import { MapFrame } from "@accessmod/ui/am/map-frame";
import { AmButton, Callout, FilePicker, InfoTip, LinkActions } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { CenterTitle } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import type maplibregl from "maplibre-gl";
import { useState } from "react";
import { api } from "../api/client";
import { keys, queryClient, useBounds, useProject, useProjects, useSettings } from "../api/queries";
import { operationDone, runResult } from "../api/tasks";
import { current, HOME, navigate, type Route } from "../app/route";
import type { Api } from "../generated/types";
import { useBatch } from "../state/batch";
import { ACCEPT } from "../state/catalogue";
import { extentDegrees, extentMetric, gridParameters, validName } from "../state/project-summary";
import { demoSlug } from "../state/start";
import { busy, useTransfer } from "../state/transfer";
import { BatchStatus, EntryList, TransferStatus, useFmt } from "./common";
import { RegionMap, RegionPanel } from "./project-region";

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
            { value: "region", icon: "map", title: "creator.title", content: <RegionPanel /> },
            { value: "delete", icon: "delete", title: "project_delete_title", content: <DeletePanel /> },
            { value: "export", icon: "export", title: "project_export_title", content: <ExportPanel route={route} /> },
          ]}
        />
      }
      right={
        route.panel === "region" ? (
          <>
            <CenterTitle title="creator.map_title" sub="creator.map_sub" />
            <RegionMap />
          </>
        ) : (
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
        )
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

/** A project file (`.am6p`) or an AccessMod 5 data archive (`.zip`, Data manager › Archive), rather than a DEM. */
const isProjectFile = (files: readonly File[]) => files.length === 1 && /\.(am6p|zip)$/i.test(files[0]!.name);

function NewPanel() {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const { state, start } = useTransfer();
  const busy = state.state === "uploading" || state.state === "processing";
  // AM5 "Choose DEM or project": a DEM creates a project, a project file or an AM5 archive imports one.
  const create = (files: File[]) =>
    start(files, async (upload, queued) => {
      const source = { upload: { id: upload.id } };
      const operation = isProjectFile(files)
        ? await api.command("project.import", { name: name.trim(), package: source })
        : await api.command("project.create", { name: name.trim(), dem: source });
      queued({ kind: "operation", id: operation.id });
      const made = await operationDone(operation.id);
      await queryClient.invalidateQueries({ queryKey: keys.projects });
      navigate({ ...HOME, project: made.project });
      return made.name;
    });
  return (
    <div className="flex flex-col gap-[15px]">
      <TextField label="project_new_name_text" value={name} onChange={setName} />
      {!validName(name) && <p className="text-xs text-muted-foreground">{t("project_new_name_unlock_dem")}</p>}
      <FilePicker block multiple k="project_import_btn" accept={`${ACCEPT.raster},.am6p,.zip`} disabled={!validName(name) || busy} onFiles={create} />
      <p className="text-xs text-muted-foreground">
        {t("project.import_supported")} {t("project.import_am5")} <InfoTip k="project.import_am5_hint" label="project.import_am5" />, {t("project.import_am6")}{" "}
        <InfoTip k="project.import_am6_hint" label="project.import_am6" />
      </p>
      <TransferStatus transfer={state} />
    </div>
  );
}

function DeletePanel() {
  const f = useFmt();
  const projects = useProjects();
  const [chosen, setChosen] = useState<string | undefined>(undefined);
  const [confirming, setConfirming] = useState(false);
  // A removal is a job: queued and ended are notified; here only a refusal.
  const { state, run } = useTransfer();
  const name = projects.data?.find((p) => p.slug === chosen)?.info.name ?? chosen ?? "";
  const remove = () => {
    if (chosen === undefined) return;
    const project = chosen;
    void run(async (queued) => {
      const operation = await api.command("project.remove", { project });
      queued({ kind: "operation", id: operation.id });
      setChosen(undefined);
      // Leave the project once it is gone (a failure is notified).
      await operationDone(operation.id);
      // The project open now, not when it was asked: another may be.
      if (current().project === project) navigate({ ...HOME, panel: "delete" });
      return name;
    });
  };
  return (
    <div className="flex flex-col gap-[15px]">
      <SelectField label="project_delete_select_project" options={projectOptions(projects.data)} value={chosen} onChange={setChosen} />
      <AmButton variant="danger" block k="project_delete_btn" disabled={chosen === undefined || busy(state)} onClick={() => setConfirming(true)} />
      <TransferStatus transfer={state} />
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} title="srv_project_confirmation_title" confirm="srv_project_delete_confirm_btn" onConfirm={remove}>
        {f("srv_project_delete_project_confirm", name)}
      </ConfirmDialog>
    </div>
  );
}

/** AM5 Project › Export: the open project as a project file (`.am6p`), downloaded. */
function ExportPanel({ route }: { route: Route }) {
  const [results, setResults] = useState(true);
  const { state, run } = useTransfer();
  const busy = state.state === "uploading" || state.state === "processing";
  const slug = route.project;
  const exportProject = (project: string) =>
    run(async (queued) => {
      const record = await api.command("project.export", { project, results });
      queued({ kind: "run", project, id: record.id });
      const file = await runResult<Api.ExportedProject>(project, record.id);
      window.location.href = api.exportUrl(project, file.name);
      return file.name;
    });
  return (
    <div className="flex flex-col gap-[15px]">
      {slug === null && <Callout k="project.open_first" />}
      <CheckboxField label="project.export_results" checked={results} onChange={setResults} />
      <AmButton block k="project_export_btn" disabled={slug === null || busy} onClick={() => slug !== null && void exportProject(slug)} />
      <TransferStatus transfer={state} />
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
