import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api/client";
import { useAnalyses, useDatasets, useJobs } from "../api/queries";
import { ANALYSIS_SCREENS, analysisName, TOOLBOX_SCREENS, t } from "../app/i18n";
import { navigate } from "../app/route";
import { EMPTY_JOB, type JobDraft, type JobMessage, updateJob } from "../documents/job";
import { SchemaForm } from "../forms/SchemaForm";
import { defaultValue, type Json, type Schema } from "../forms/schema";
import type { Api } from "../generated/types";
import { createStore, type HistoryMessage, history, useStore, withHistory } from "../state/store";
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
  // The document the user asked the server to check (a user message).
  const [requested, setRequested] = useState<string | null>(null);
  const documentKey = JSON.stringify(draft.document);

  // Every change of the document is a message; it also drops the check of
  // the previous document, whose request is then cancelled.
  const dispatch = (message: HistoryMessage<JobMessage>) => {
    store.dispatch(message);
    setRequested(null);
  };

  // Server check: a query keyed by the document it checks.
  const validation = useQuery({
    queryKey: ["validate", project, analysis, requested],
    queryFn: ({ signal }) =>
      api.validate(project, analysis, JSON.parse(requested ?? "null") as Json, signal),
    enabled: requested !== null,
    retry: false,
    staleTime: 0,
  });
  const check = requested === documentKey ? validation : null;

  const save = useMutation({
    mutationFn: (snapshot: JobDraft) =>
      api.command<Api.SavedJob>("job.save", {
        project,
        analysis,
        name: snapshot.name || title,
        document: snapshot.document,
        ...(snapshot.saved ? { id: snapshot.saved.id, revision: snapshot.saved.revision } : {}),
      }),
    onSuccess: (job, snapshot) => {
      const now = store.getState().present;
      if (now === snapshot) {
        store.dispatch({ type: "do", message: { type: "saved", job } });
      } else if (now.saved?.id === snapshot.saved?.id) {
        // Edited while saving: keep the edits, record the saved revision.
        store.dispatch({
          type: "do",
          message: { type: "revised", id: job.id, revision: job.revision },
        });
      }
      // Another job was loaded meanwhile: nothing to update.
    },
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
          aria-label={t("job.saved")}
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
          aria-label={t("job.name")}
          placeholder={t("job.name")}
          value={draft.name}
          onChange={(e) =>
            dispatch({ type: "do", message: { type: "rename", name: e.target.value } })
          }
        />
        <button type="button" disabled={save.isPending} onClick={() => save.mutate(draft)}>
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
            disabled={check?.isFetching === true}
            onClick={() => {
              setRequested(documentKey);
              if (requested === documentKey) void validation.refetch();
            }}
          >
            {check?.isFetching ? t("common.loading") : t("job.validate")}
          </button>
          {check?.data && <Validation result={check.data} />}
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
