import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api/client";
import { useAnalyses, useDatasets, useJobs } from "../api/queries";
import { ANALYSIS_SCREENS, analysisName, TOOLBOX_SCREENS, t } from "../app/i18n";
import { navigate } from "../app/route";
import { EMPTY_JOB, type JobDraft, updateJob } from "../documents/job";
import { SchemaForm } from "../forms/SchemaForm";
import { defaultValue, type Json, type Schema } from "../forms/schema";
import type { Api } from "../generated/types";
import { createStore, history, useStore, withHistory } from "../state/store";
import { Issues, Loading } from "./common";

export function AnalysisScreen({
  project,
  section,
  analysis,
  title,
}: {
  project: string;
  section: "analysis" | "toolbox";
  analysis: string | null;
  title: string | null;
}) {
  const analyses = useAnalyses();
  const screens = section === "analysis" ? ANALYSIS_SCREENS : TOOLBOX_SCREENS;
  if (analysis === null) {
    return (
      <section>
        <h1>{t(`section.${section}`)}</h1>
        <div className="cards">
          {screens.map((name) => (
            <button
              type="button"
              className="card"
              key={name}
              onClick={() => navigate({ project, section, item: name })}
            >
              <strong>{analysisName(name)}</strong>
              <span className="muted small">{name}</span>
            </button>
          ))}
        </div>
      </section>
    );
  }
  const info = analyses.data?.find((a) => a.name === analysis);
  if (analyses.isPending) return <Loading />;
  if (!info) return <Issues error={analyses.error ?? `no analysis ${analysis}`} />;
  return (
    <JobEditor
      project={project}
      analysis={analysis}
      schema={info.schema as Schema}
      title={title ?? analysis}
    />
  );
}

/** A new document of `schema`: its required fields, at schema version 1. */
function newDocument(schema: Schema): Json {
  const value = defaultValue(schema, schema);
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? { ...value, schema_version: 1 }
    : value;
}

function draftStore(analysis: string, schema: Schema) {
  const initial: JobDraft = updateJob(EMPTY_JOB, {
    type: "new",
    analysis,
    document: newDocument(schema),
  });
  return createStore(history(initial), withHistory(updateJob));
}

/** What the server said about a document, and which document it was. */
interface Checked {
  document: string;
  result: Record<string, unknown> | null;
  error: unknown;
}

function JobEditor({
  project,
  analysis,
  schema,
  title,
}: {
  project: string;
  analysis: string;
  schema: Schema;
  title: string;
}) {
  // One store per editor: the document being edited.
  const [store] = useState(() => draftStore(analysis, schema));
  const draft = useStore(store, (s) => s.present);
  const canUndo = useStore(store, (s) => s.past.length > 0);
  const canRedo = useStore(store, (s) => s.future.length > 0);
  const datasets = useDatasets(project);
  const jobs = useJobs(project);
  const [tags, setTags] = useState("");
  const [checked, setChecked] = useState<Checked | null>(null);
  const documentKey = JSON.stringify(draft.document);
  // A check applies only to the document it checked (derived, not stored).
  const check = checked?.document === documentKey ? checked : null;

  const dispatch = store.dispatch;
  const save = useMutation({
    mutationFn: () =>
      api.command<Api.SavedJob>("job.save", {
        project,
        analysis,
        name: draft.name || title,
        document: draft.document,
        ...(draft.saved ? { id: draft.saved.id, revision: draft.saved.revision } : {}),
      }),
    onSuccess: (job) => dispatch({ type: "do", message: { type: "saved", job } }),
  });
  const validate = useMutation({
    mutationFn: (document: Json) => api.validate(project, analysis, document),
    onSuccess: (result, document) =>
      setChecked({ document: JSON.stringify(document), result, error: null }),
    onError: (error, document) =>
      setChecked({ document: JSON.stringify(document), result: null, error }),
  });
  const run = useMutation({
    mutationFn: () =>
      api.command<Api.RunRecord>("run.start", {
        project,
        analysis,
        document: draft.document,
        tags: tags.split(/[\s,;]+/).filter((x) => x.length > 0),
      }),
  });
  const saved = jobs.data?.filter((j) => j.analysis === analysis) ?? [];

  return (
    <section>
      <h1>{title}</h1>
      <div className="toolbar">
        <select
          value={draft.saved?.id ?? ""}
          onChange={(e) => {
            const job = saved.find((j) => j.id === Number(e.target.value));
            if (job) dispatch({ type: "do", message: { type: "load", job } });
          }}
        >
          <option value="">{t("job.saved")}…</option>
          {saved.map((j) => (
            <option key={j.id} value={j.id}>
              {j.name} (#{j.id})
            </option>
          ))}
        </select>
        <button
          type="button"
          onClick={() =>
            dispatch({
              type: "do",
              message: { type: "new", analysis, document: newDocument(schema) },
            })
          }
        >
          {t("job.new")}
        </button>
        <input
          placeholder={t("job.name")}
          value={draft.name}
          onChange={(e) =>
            dispatch({ type: "do", message: { type: "rename", name: e.target.value } })
          }
        />
        <button type="button" disabled={save.isPending} onClick={() => save.mutate()}>
          {t("job.save")}
          {draft.dirty ? " *" : ""}
        </button>
        <button type="button" disabled={!canUndo} onClick={() => dispatch({ type: "undo" })}>
          {t("job.undo")}
        </button>
        <button type="button" disabled={!canRedo} onClick={() => dispatch({ type: "redo" })}>
          {t("job.redo")}
        </button>
      </div>
      <Issues error={save.error} />
      <div className="editor">
        <SchemaForm
          root={schema}
          value={draft.document}
          datasets={datasets.data ?? []}
          dispatch={(message) => dispatch({ type: "do", message })}
        />
        <aside className="actions">
          <button
            type="button"
            disabled={validate.isPending}
            onClick={() => validate.mutate(draft.document)}
          >
            {validate.isPending ? t("common.loading") : t("job.validate")}
          </button>
          {check?.result && <Validation result={check.result} />}
          {check?.error ? <Issues error={check.error} /> : null}
          <label>
            {t("job.tags")}
            <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder={analysis} />
          </label>
          <button
            type="button"
            className="primary"
            disabled={run.isPending}
            onClick={() => run.mutate()}
          >
            {t("job.run")}
          </button>
          {run.isSuccess && (
            <p className="ok">
              {t("job.queued")}{" "}
              <button
                type="button"
                className="link"
                onClick={() => navigate({ project, section: "logs", item: null })}
              >
                {t("section.logs")}
              </button>
            </p>
          )}
          <Issues error={run.error} />
        </aside>
      </div>
    </section>
  );
}

function Validation({ result }: { result: Record<string, unknown> }) {
  const warnings = Array.isArray(result.warnings)
    ? (result.warnings as { message?: string }[])
    : [];
  return (
    <div className="ok">
      <strong>{t("job.valid")}</strong>
      {warnings.length > 0 && (
        <ul className="warnings">
          {warnings.map((w) => (
            <li key={w.message}>{w.message}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
