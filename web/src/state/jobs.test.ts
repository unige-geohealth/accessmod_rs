import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { duration, jobKey, jobKind, jobRows, newestFirst, ofKind, pendingJobs, queuePlaces, seconds, summary } from "./jobs";

const run = (id: number, status: Api.RunStatus, started: number, finished: number | null, extra: Partial<Api.RunRecord> = {}) =>
  ({ id, analysis: "travel-time", tags: ["dry"], job: {}, status, output_dir: `runs/${id}`, started, finished, datasets: [], ...extra }) as Api.RunRecord;
const entry = (project: string, r: Api.RunRecord): Api.JobEntry => ({ project, name: project.toUpperCase(), run: r });

const entries = [
  entry("a", run(3, "running", 1000, null)),
  entry("b", run(1, "queued", 990, null)),
  entry("a", run(2, "failed", 900, 905, { error: { kind: "validation", issues: ["no land cover"] } })),
  entry("a", run(1, "done", 800, 875, { outcome: "12 cells" })),
];

describe("jobs", () => {
  it("summarises the queue and the computing time", () => {
    expect(summary(entries)).toEqual({ total: 4, running: 1, queued: 1, failed: 1, computing: 80 });
  });

  it("formats durations", () => {
    expect([duration(12), duration(185), duration(3720)]).toEqual(["12 s", "3 min 05 s", "1 h 02 min"]);
    expect(seconds(entries[0]!.run, 1030)).toBe(30);
    expect(seconds(entries[1]!.run, 1030)).toBe(40);
  });

  it("lists rows with the stage of the running job and the outcome or first issue", () => {
    const t = (k: string) => k;
    const progress = { [jobKey(entries[0]!)]: { stage: "solve", done: 1, total: 4 } as Api.Progress };
    const rows = jobRows(entries, t as never, progress, 1030, (s) => String(s));
    expect(rows.map((r) => [r.key, r.project, r.status, r.duration, r.outcome])).toEqual([
      ["a/3", "A", "jobs.status_running: task.stage_solve 25 %", "30 s", ""],
      ["b/1", "B", "jobs.status_queued", "40 s", ""],
      ["a/2", "A", "jobs.status_failed", "5 s", "no land cover"],
      ["a/1", "A", "jobs.status_done", "1 min 15 s", "12 cells"],
    ]);
    expect(rows[0]!.analysis).toBe("analysis_accessibility_main_tab");
  });
  it("lists operations no project records (a removal stays), and filters by kind", () => {
    const op = (id: number, status: Api.RunStatus, recorded = false): Api.PendingOperation => ({
      recorded,
      id,
      operation: "project-import",
      project: `p${id}`,
      name: `P${id}`,
      owner: "local",
      status,
      started: 100 + id,
    });
    const pending = pendingJobs([op(1, "running"), op(2, "done", true), op(3, "failed")]);
    // A removal done is recorded nowhere but here.
    expect(pendingJobs([{ ...op(4, "done"), operation: "project-remove" }]).map(jobKey)).toEqual([":pending/4"]);
    expect(pending.map((e) => [jobKey(e), e.run.status, e.run.kind])).toEqual([
      [":pending/3", "failed", "manage"],
      [":pending/1", "running", "manage"],
    ]);
    const archive = entry("a", run(9, "done", 1, 2, { kind: "manage", analysis: "archive" }));
    const mixed = [...pending, archive, ...entries];
    expect(ofKind(mixed, "manage").length).toBe(3);
    expect(ofKind(mixed, "analysis").length).toBe(entries.length);
    expect(ofKind(mixed, jobKind("nonsense")).length).toBe(mixed.length);
    // Pending ones among the runs, by start.
    expect(newestFirst(mixed).map((e) => e.run.started)).toEqual([1000, 990, 900, 800, 103, 101, 1]);
  });

  it("shows the place in the queue and a pause", () => {
    const t = (k: string) => k;
    const places = queuePlaces([
      { lane: "compute", project: "a", run: 3, position: 0, paused: true },
      { lane: "compute", project: "b", run: 1, position: 2, paused: false },
      { lane: "io", project: ":pending", run: 4, position: 1, paused: false },
    ]);
    expect(Object.keys(places).sort()).toEqual([":pending/4", "a/3", "b/1"]);
    const rows = jobRows(entries.slice(0, 2), t as never, {}, 1030, String, places);
    expect(rows.map((r) => r.status)).toEqual(["jobs.status_running (task.paused)", "jobs.status_queued (#2)"]);
  });
});
