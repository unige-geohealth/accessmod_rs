import { describe, expect, it } from "vitest";
import { history, withHistory } from "../state/store";
import { EMPTY_JOB, updateJob } from "./job";

describe("job draft", () => {
  it("edits, appends and removes through messages", () => {
    let s = updateJob(EMPTY_JOB, {
      type: "new",
      analysis: "travel-time",
      document: { scenario: [] },
    });
    expect(s.dirty).toBe(false);
    s = updateJob(s, { type: "append", path: ["scenario"], value: { class: 1 } });
    s = updateJob(s, { type: "append", path: ["scenario"], value: { class: 2 } });
    s = updateJob(s, { type: "set", path: ["scenario", 1, "class"], value: 3 });
    s = updateJob(s, { type: "remove", path: ["scenario", 0] });
    expect(s.document).toEqual({ scenario: [{ class: 3 }] });
    expect(s.dirty).toBe(true);
  });

  it("keeps the saved revision and clears dirty on save", () => {
    const job = {
      id: 4,
      revision: 2,
      analysis: "referral",
      name: "a",
      document: { x: 1 },
      updated: 0,
    };
    let s = updateJob(EMPTY_JOB, { type: "load", job });
    expect(s.saved).toEqual({ id: 4, revision: 2 });
    s = updateJob(s, { type: "rename", name: "b" });
    expect(s.dirty).toBe(true);
    s = updateJob(s, { type: "saved", job: { ...job, name: "b", revision: 3 } });
    expect(s).toMatchObject({ dirty: false, saved: { id: 4, revision: 3 }, name: "b" });
  });

  it("undoes edits", () => {
    const step = withHistory(updateJob);
    let h = history(updateJob(EMPTY_JOB, { type: "new", analysis: "t", document: {} }));
    h = step(h, { type: "do", message: { type: "set", path: ["a"], value: 1 } });
    h = step(h, { type: "undo" });
    expect(h.present.document).toEqual({});
  });
});

describe("late saves", () => {
  it("keep the edits made meanwhile", () => {
    let s = updateJob(EMPTY_JOB, { type: "new", analysis: "t", document: { a: 1 } });
    s = updateJob(s, { type: "set", path: ["a"], value: 2 });
    s = updateJob(s, { type: "revised", id: 9, revision: 1 });
    expect(s).toMatchObject({ document: { a: 2 }, dirty: true, saved: { id: 9, revision: 1 } });
  });
});
