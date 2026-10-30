import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { busy, lastResults, minutes, percent, runView } from "./run";

const run = (id: number, status: Api.RunStatus, datasets: number[] = []): Api.RunRecord =>
  ({ id, status, datasets, started: 100, finished: status === "running" ? null : 130, analysis: "travel-time", tags: [], job: {}, output_dir: "" }) as Api.RunRecord;

describe("run", () => {
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
    expect(lastResults([run(1, "done", [5]), run(3, "failed", [9]), run(2, "done", [6, 7])])).toEqual([6, 7]);
    expect(lastResults(undefined)).toEqual([]);
  });
});
