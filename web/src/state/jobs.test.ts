import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { duration, jobKey, jobRows, seconds, summary } from "./jobs";

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
      ["a/3", "A", "jobs.status_running: solve 25 %", "30 s", ""],
      ["b/1", "B", "jobs.status_queued", "40 s", ""],
      ["a/2", "A", "jobs.status_failed", "5 s", "no land cover"],
      ["a/1", "A", "jobs.status_done", "1 min 15 s", "12 cells"],
    ]);
    expect(rows[0]!.analysis).toBe("analysis_accessibility_main_tab");
  });
});
