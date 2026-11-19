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
    // Newest first.
    expect(visibleNotices(state, 101).map((n) => n.text)).toEqual(["disk full", "archived as a.zip"]);
    expect(visibleNotices(state, 100 + FADE_S).map((n) => n.text)).toEqual(["disk full"]);
    state = updateNotifications(state, { type: "dismiss", id: failed.id });
    expect(visibleNotices(state, 101).map((n) => n.id)).toEqual([done.id]);
    expect(updateNotifications(state, { type: "dismiss", id: "nope" })).toBe(state);
  });

  it("notices a queued job, which fades like a success", () => {
    const queued = runNotice("p", run(3, "queued", { analysis: "travel-time", kind: "analysis", tags: ["scenario", "b"] }), 50)!;
    expect([queued.id, queued.status, queued.text]).toEqual(["p/3/queued", "queued", "scenario b"]);
    const state = updateNotifications(NO_NOTIFICATIONS, { type: "notice", notice: queued });
    expect(visibleNotices(state, 51).map((n) => n.id)).toEqual([queued.id]);
    expect(visibleNotices(state, 50 + FADE_S)).toEqual([]);
    // Its end is another notice.
    const done = runNotice("p", run(3, "done", { analysis: "travel-time", kind: "analysis" }), 60)!;
    expect(updateNotifications(state, { type: "notice", notice: done }).notices.length).toBe(2);
    expect(updateNotifications(state, { type: "notice", notice: { ...queued, id: "x", status: "running" } })).toBe(state);
  });

  it("notices operations making projects apart from runs", () => {
    const op = operationNotice(
      { id: 1, operation: "project-import", project: "x", name: "X", owner: "local", status: "done", started: 0 },
      5,
    )!;
    expect([op.id, op.pending, op.text]).toEqual([":pending/1/done", true, "X"]);
    // Queued too, as every job; never while running.
    const base = { id: 2, operation: "project-create", project: "y", name: "Y", owner: "local", started: 0 } as const;
    expect(operationNotice({ ...base, status: "queued" }, 5)?.id).toBe(":pending/2/queued");
    expect(operationNotice({ ...base, status: "running" }, 5)).toBeNull();
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
