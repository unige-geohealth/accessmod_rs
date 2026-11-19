import { describe, expect, it } from "vitest";
import { IDLE, type Transfer, type TransferMessage, updateTransfer } from "./transfer";

const run = (messages: TransferMessage[], from: Transfer = IDLE) => messages.reduce(updateTransfer, from);

describe("transfer", () => {
  it("goes through upload and processing to done", () => {
    expect(run([{ type: "start" }, { type: "progress", fraction: 0.5 }])).toEqual({ state: "uploading", fraction: 0.5 });
    expect(run([{ type: "start" }, { type: "uploaded" }, { type: "done" }])).toEqual({ state: "done" });
  });

  it("follows the task the command queued", () => {
    const task = { kind: "run" as const, project: "p", id: 4 };
    expect(run([{ type: "start" }, { type: "uploaded" }])).toEqual({ state: "processing", task: null });
    expect(run([{ type: "start" }, { type: "uploaded" }, { type: "queued", task }])).toEqual({ state: "processing", task });
    expect(run([{ type: "start" }, { type: "queued", task }])).toEqual({ state: "uploading", fraction: 0 });
  });

  it("ignores messages that do not fit the state", () => {
    expect(run([{ type: "progress", fraction: 1 }])).toBe(IDLE);
    expect(run([{ type: "done" }])).toBe(IDLE);
    const uploading = run([{ type: "start" }]);
    expect(updateTransfer(uploading, { type: "start" })).toBe(uploading);
    expect(updateTransfer(uploading, { type: "reset" })).toBe(uploading);
  });

  it("clamps progress and keeps failures", () => {
    expect(run([{ type: "start" }, { type: "progress", fraction: 3 }])).toEqual({ state: "uploading", fraction: 1 });
    const failed = run([{ type: "start" }, { type: "failed", issues: ["no"] }]);
    expect(failed).toEqual({ state: "failed", issues: ["no"] });
    expect(updateTransfer(failed, { type: "reset" })).toBe(IDLE);
  });

  it("leaves the failure of a queued job to its notification", () => {
    const task = { kind: "operation" as const, id: 2 };
    expect(run([{ type: "start" }, { type: "uploaded" }, { type: "queued", task }, { type: "failed", issues: ["busy"] }])).toBe(IDLE);
  });
});
