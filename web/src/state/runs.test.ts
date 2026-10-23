import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { fraction, NO_RUNS, updateRuns } from "./runs";

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

describe("runs", () => {
  it("follows a run from queued to done through events", () => {
    let s = updateRuns(NO_RUNS, { type: "open", project: "p" });
    s = updateRuns(s, {
      type: "event",
      event: { type: "run", project: "p", run: record(1, "queued") },
    });
    expect(s.runs[1]?.status).toBe("queued");
    s = updateRuns(s, {
      type: "event",
      event: {
        type: "progress",
        project: "p",
        run: 1,
        progress: { stage: "solve", done: 5, total: 10 },
      },
    });
    const running = s.runs[1];
    expect(running?.status).toBe("running");
    expect(running && fraction(running)).toBe(0.5);
    s = updateRuns(s, {
      type: "event",
      event: { type: "run", project: "p", run: record(1, "done") },
    });
    expect(s.runs[1]?.status).toBe("done");
    // Late progress does not revive a finished run.
    const after = updateRuns(s, {
      type: "event",
      event: { type: "progress", project: "p", run: 1, progress: { stage: "solve", done: 9 } },
    });
    expect(after).toBe(s);
  });

  it("ignores other projects and keeps live progress on reload", () => {
    let s = updateRuns(NO_RUNS, { type: "open", project: "p" });
    const other = updateRuns(s, {
      type: "event",
      event: { type: "run", project: "q", run: record(2, "queued") },
    });
    expect(other).toBe(s);
    s = updateRuns(s, { type: "loaded", project: "p", runs: [record(3, "running")] });
    s = updateRuns(s, {
      type: "event",
      event: {
        type: "progress",
        project: "p",
        run: 3,
        progress: { stage: "solve", done: 1, total: 4 },
      },
    });
    s = updateRuns(s, { type: "loaded", project: "p", runs: [record(3, "running")] });
    const live = s.runs[3];
    expect(live?.status === "running" && live.progress?.done).toBe(1);
  });
});
