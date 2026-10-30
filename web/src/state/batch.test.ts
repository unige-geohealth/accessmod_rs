import { describe, expect, it } from "vitest";
import { type BatchMessage, INITIAL, updateBatch } from "./batch";

const run = (messages: BatchMessage[]) => messages.reduce(updateBatch, INITIAL).batch;
const ok = (label: string): BatchMessage => ({ type: "step", outcome: { label, issues: null } });

describe("batch", () => {
  it("counts steps and lists every outcome", () => {
    expect(run([{ type: "start", total: 2 }, ok("a")])).toEqual({ state: "running", done: 1, total: 2 });
    const failed: BatchMessage = { type: "step", outcome: { label: "b", issues: ["no"] } };
    expect(run([{ type: "start", total: 2 }, ok("a"), failed])).toEqual({
      state: "done",
      outcomes: [
        { label: "a", issues: null },
        { label: "b", issues: ["no"] },
      ],
    });
  });

  it("ignores steps outside a run and resets only when idle or done", () => {
    expect(run([ok("a")])).toEqual({ state: "idle" });
    expect(run([{ type: "start", total: 1 }, { type: "reset" }]).state).toBe("running");
    expect(run([{ type: "start", total: 0 }])).toEqual({ state: "done", outcomes: [] });
  });
});
