import { describe, expect, it } from "vitest";
import { CLOSED, isNoticeOf, type JobPanel, type JobPanelMessage, updateJobPanel } from "./job-panel";

const run = (messages: JobPanelMessage[], from: JobPanel = CLOSED) => messages.reduce(updateJobPanel, from);
const op = { kind: "operation", id: 3 } as const;
const analysis = { kind: "run", project: "p", id: 7 } as const;

describe("job panel", () => {
  it("shows an upload, then its job", () => {
    expect(run([{ type: "upload", upload: 1 }, { type: "progress", upload: 1, fraction: 2 }])).toEqual({ kind: "upload", upload: 1, fraction: 1 });
    expect(run([{ type: "upload", upload: 1 }, { type: "follow", task: op, upload: 1 }])).toEqual({ kind: "job", task: op });
  });

  it("stays closed for the job of an upload closed while sending", () => {
    const closed = run([{ type: "upload", upload: 1 }, { type: "close" }]);
    expect(run([{ type: "progress", upload: 1, fraction: 0.5 }], closed)).toEqual(closed);
    expect(run([{ type: "follow", task: op, upload: 1 }], closed)).toEqual(CLOSED);
    // Nor does it take the panel from a later upload.
    expect(run([{ type: "upload", upload: 2 }, { type: "follow", task: op, upload: 1 }], closed)).toEqual({ kind: "upload", upload: 2, fraction: 0 });
    // Another job, started afterwards, opens it.
    expect(run([{ type: "follow", task: analysis }], closed)).toEqual({ kind: "job", task: analysis });
  });

  it("lets a new job take over and closes on a failed upload", () => {
    expect(run([{ type: "follow", task: op }, { type: "follow", task: analysis }])).toEqual({ kind: "job", task: analysis });
    expect(run([{ type: "upload", upload: 2 }, { type: "drop", upload: 2 }])).toEqual(CLOSED);
    // A failure of an older upload leaves the panel alone.
    expect(run([{ type: "follow", task: op }, { type: "drop", upload: 2 }])).toEqual({ kind: "job", task: op });
  });

  it("matches the notices of a job", () => {
    expect(isNoticeOf({ pending: true, project: "x", run: 3 }, op)).toBe(true);
    expect(isNoticeOf({ pending: false, project: "x", run: 3 }, op)).toBe(false);
    expect(isNoticeOf({ pending: false, project: "p", run: 7 }, analysis)).toBe(true);
    expect(isNoticeOf({ pending: false, project: "q", run: 7 }, analysis)).toBe(false);
  });
});
