import { describe, expect, it } from "vitest";
import type { Api } from "../generated/types";
import { FADE_S, NO_NOTIFICATIONS, operationNotice, runNotice, updateNotifications, visibleNotices } from "./notifications";

const run = (id: number, status: Api.RunStatus, extra: Partial<Api.RunRecord> = {}) =>
  ({ id, analysis: "archive", kind: "manage", tags: [], job: {}, status, output_dir: "", started: 0, datasets: [], inputs: [], ...extra }) as Api.RunRecord;

describe("notifications", () => {
  it("notices ended jobs once, fades successes and keeps failures until closed", () => {
    expect(runNotice("p", run(1, "running"), 0)).toBeNull();
    const done = runNotice("p", run(1, "done", { outcome: "archived as a.zip" }), 100)!;
    const failed = runNotice("p", run(2, "failed", { error: { kind: "io", issues: ["disk full"] } }), 100)!;
    let state = updateNotifications(NO_NOTIFICATIONS, { type: "notice", notice: done });
    state = updateNotifications(state, { type: "notice", notice: failed });
    expect(updateNotifications(state, { type: "notice", notice: done })).toBe(state);
    expect(visibleNotices(state, 101).map((n) => n.text)).toEqual(["archived as a.zip", "disk full"]);
    expect(visibleNotices(state, 100 + FADE_S).map((n) => n.text)).toEqual(["disk full"]);
    state = updateNotifications(state, { type: "dismiss", id: failed.id });
    expect(visibleNotices(state, 101).map((n) => n.id)).toEqual([done.id]);
    expect(updateNotifications(state, { type: "dismiss", id: "nope" })).toBe(state);
  });

  it("notices operations making projects apart from runs", () => {
    const op = operationNotice(
      { id: 1, operation: "project-import", project: "x", name: "X", owner: "local", status: "done", started: 0 },
      5,
    )!;
    expect([op.id, op.pending, op.text]).toEqual([":pending/1/done", true, "X"]);
  });

  it("keeps a bounded list", () => {
    let state = NO_NOTIFICATIONS;
    for (let i = 0; i < 30; i++) state = updateNotifications(state, { type: "notice", notice: runNotice("p", run(i, "done"), 0)! });
    expect(state.notices.length).toBe(20);
    // A failure outlives later successes.
    state = updateNotifications(NO_NOTIFICATIONS, { type: "notice", notice: runNotice("p", run(99, "failed"), 0)! });
    for (let i = 0; i < 30; i++) state = updateNotifications(state, { type: "notice", notice: runNotice("p", run(i, "done"), 0)! });
    expect(state.notices.some((n) => n.status === "failed")).toBe(true);
  });
});
