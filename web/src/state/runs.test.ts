import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { fraction, NO_RUNS, showRun, updateRuns } from "./runs";

const record = (id: number, status: Api.RunStatus): Api.RunRecord => ({
  id,
  analysis: "travel-time",
  tags: ["t"],
  job: {},
  status,
  output_dir: `runs/${id}`,
  started: 0,
  datasets: [],
});
const event = (e: Api.Event) => ({ type: "event" as const, event: e });

describe("runs", () => {
  it("follows a run through events, per project", () => {
    let s = updateRuns(NO_RUNS, event({ type: "run", project: "p", run: record(1, "queued") }));
    s = updateRuns(
      s,
      event({
        type: "progress",
        project: "p",
        run: 1,
        progress: { stage: "solve", done: 5, total: 10 },
      }),
    );
    let shown = showRun(record(1, "queued"), s.runs.p, s.progress.p);
    expect(shown.run.status).toBe("running");
    expect(fraction(shown.progress)).toBe(0.5);
    s = updateRuns(s, event({ type: "run", project: "p", run: record(1, "done") }));
    shown = showRun(record(1, "running"), s.runs.p, s.progress.p);
    expect(shown).toEqual({ run: record(1, "done"), progress: null });
    // Late progress does not revive a finished run.
    const late = updateRuns(
      s,
      event({ type: "progress", project: "p", run: 1, progress: { stage: "x", done: 1 } }),
    );
    expect(late).toBe(s);
    // Other projects are apart.
    expect(s.runs.q).toBeUndefined();
  });

  it("shows progress of runs started before the page", () => {
    const s = updateRuns(
      NO_RUNS,
      event({
        type: "progress",
        project: "p",
        run: 7,
        progress: { stage: "solve", done: 1, total: 4 },
      }),
    );
    const shown = showRun(record(7, "running"), s.runs.p, s.progress.p);
    expect(shown.progress?.done).toBe(1);
  });

  it("trusts finished records over stale live states", () => {
    const s = updateRuns(NO_RUNS, event({ type: "run", project: "p", run: record(2, "running") }));
    expect(showRun(record(2, "failed"), s.runs.p, s.progress.p).run.status).toBe("failed");
  });
});
