import { describe, expect, it } from "vitest";
import { IDLE, type Transfer, type TransferMessage, updateTransfer } from "./transfer";

const run = (messages: TransferMessage[], from: Transfer = IDLE) => messages.reduce(updateTransfer, from);

describe("transfer", () => {
  it("goes through upload and processing to done", () => {
    expect(run([{ type: "start" }, { type: "progress", fraction: 0.5 }])).toEqual({ state: "uploading", fraction: 0.5 });
    expect(run([{ type: "start" }, { type: "uploaded" }, { type: "done", label: "x" }])).toEqual({ state: "done", label: "x" });
  });

  it("ignores messages that do not fit the state", () => {
    expect(run([{ type: "progress", fraction: 1 }])).toBe(IDLE);
    expect(run([{ type: "done", label: "x" }])).toBe(IDLE);
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
});
