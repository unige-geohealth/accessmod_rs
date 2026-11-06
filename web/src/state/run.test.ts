import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { updateSession } from "./drafts";
import { busy, doneRuns, lastRun, minutes, percent, runView, stageLabel } from "./run";

const run = (id: number, status: Api.RunStatus, datasets: number[] = []): Api.RunRecord =>
  ({ id, status, datasets, inputs: [], started: 100, finished: status === "running" ? null : 130, analysis: "travel-time", tags: [], job: {}, output_dir: "" }) as Api.RunRecord;

describe("run", () => {
  it("keeps the session starting from Compute until the run is queued or refused", () => {
    const idle = { run: null, acknowledged: true, starting: false };
    const starting = updateSession(idle, { type: "starting" });
    expect(starting.starting).toBe(true);
    expect(updateSession(starting, { type: "started", run: 7 })).toEqual({ run: 7, acknowledged: false, starting: false });
    expect(updateSession(starting, { type: "start_failed" })).toEqual(idle);
  });

  it("labels the engine's stages, and shows an unknown one as it is", () => {
    const t = (key: string) => `<${key}>`;
    expect(stageLabel(t, "load_inputs")).toBe("<task.stage_load_inputs>");
    expect(stageLabel(t, "archive")).toBe("<task.stage_archive>");
    expect(stageLabel(t, "something_new")).toBe("something_new");
  });

  it("follows the run the screen started", () => {
    expect(runView(undefined, undefined, null)).toEqual({ state: "idle" });
    expect(runView([], undefined, 4)).toEqual({ state: "queued" });
    const progress = { stage: "solve", done: 1, total: 4 };
    const view = runView([run(4, "running")], { 4: progress }, 4);
    expect(view).toEqual({ state: "running", progress });
    expect(busy(view)).toBe(true);
    expect(runView([run(4, "cancelled")], undefined, 4).state).toBe("failed");
    expect(runView([run(4, "done")], undefined, 4).state).toBe("done");
  });

  it("gives percents, minutes and the last results", () => {
    expect(percent({ stage: "s", done: 1, total: 3 })).toBe(33);
    expect(percent({ stage: "s", done: 1 })).toBeUndefined();
    expect(minutes(run(1, "done"))).toBe(0.5);
    const runs = [run(1, "done", [5]), run(3, "failed", [9]), run(2, "done", [6, 7])];
    expect(lastRun(runs)).toBe(2);
    expect(doneRuns(runs).map((r) => r.id)).toEqual([2, 1]);
    expect(lastRun(undefined)).toBeNull();
    // An operation (an archive) is not a job of Data.
    expect(lastRun([...runs, { ...run(8, "done"), kind: "manage" as const }])).toBe(2);
  });
});
