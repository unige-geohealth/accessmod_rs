import { useMutation } from "@tanstack/react-query";
import { api } from "../api/client";
import { runsStore } from "../api/events";
import { useRunList } from "../api/queries";
import { analysisName, t } from "../app/i18n";
import { navigate } from "../app/route";
import { fraction, type RunProcess } from "../state/runs";
import { useStore } from "../state/store";
import { Issues, Loading, when } from "./common";

export function LogsScreen({ project }: { project: string }) {
  const runs = useRunList(project);
  const live = useStore(runsStore, (s) => s.runs);
  const cancel = useMutation({
    mutationFn: (id: number) => api.command("run.cancel", { project, id }),
  });
  const list = [...(runs.data ?? [])].reverse();
  return (
    <section>
      <h1>{t("section.logs")}</h1>
      {runs.isPending && <Loading />}
      <Issues error={runs.error ?? cancel.error} />
      {list.length === 0 && !runs.isPending && <p className="muted">{t("logs.empty")}</p>}
      <table className="table">
        <tbody>
          {list.map((record) => {
            // Live state from events, while the server lists the run as queued or
            // running; a finished record always wins (derived, never stored).
            const active = record.status === "queued" || record.status === "running";
            const process: RunProcess | undefined = active ? live[record.id] : undefined;
            const status = process?.status ?? record.status;
            const run = process?.run ?? record;
            const share = process ? fraction(process) : null;
            const stage = process?.status === "running" ? process.progress?.stage : undefined;
            return (
              <tr key={record.id}>
                <td>#{run.id}</td>
                <td>{analysisName(run.analysis)}</td>
                <td>
                  <span className={`status ${status}`}>{status}</span>
                  {stage && <span className="muted small"> {stage}</span>}
                  {share !== null && <progress max={1} value={share} aria-label="progress" />}
                </td>
                <td className="small">
                  {run.outcome}
                  {run.error && (
                    <ul className="issues-inline">
                      {run.error.issues.map((i) => (
                        <li key={i}>{i}</li>
                      ))}
                    </ul>
                  )}
                  {run.datasets.length > 0 && (
                    <div>
                      {run.datasets.map((id) => (
                        <button
                          type="button"
                          className="link small"
                          key={id}
                          onClick={() => navigate({ project, section: "data", item: String(id) })}
                        >
                          #{id}
                        </button>
                      ))}
                    </div>
                  )}
                </td>
                <td className="muted small">{when(run.started)}</td>
                <td>
                  {(status === "queued" || status === "running") && (
                    <button type="button" onClick={() => cancel.mutate(run.id)}>
                      {t("logs.cancel")}
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}
