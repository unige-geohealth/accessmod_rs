// Long commands (imports, exports, archives) answer at once with the task
// they queued: a run of the project, or an operation making a project.
// These promises settle when that task ends, from events (events.ts) or,
// should an event be missed, from a slow poll. Not state: the click handler
// that started the task awaits them.
import type { Api } from "../generated/types";
import { api, ApiError } from "./client";

/** A task a command queued. */
export type TaskRef = { kind: "run"; project: string; id: number } | { kind: "operation"; id: number };

const POLL_MS = 3000;

const finished = (status: Api.RunStatus) => status !== "queued" && status !== "running";

const runWaiters = new Map<string, ((run: Api.RunRecord) => void)[]>();
const operationWaiters = new Map<number, ((operation: Api.PendingOperation) => void)[]>();

const checkWaiters = new Map<number, ((check: Api.Check) => void)[]>();

const runKey = (project: string, id: number) => `${project}\u0000${id}`;

/** An event: `run` of `project` as it is now. */
export function settleRun(project: string, run: Api.RunRecord) {
  if (!finished(run.status)) return;
  const key = runKey(project, run.id);
  const waiters = runWaiters.get(key);
  runWaiters.delete(key);
  for (const resolve of waiters ?? []) resolve(run);
}

/** An event: pre-analysis `check` as it is now. */
export function settleCheck(check: Api.Check) {
  if (!finished(check.status)) return;
  const waiters = checkWaiters.get(check.id);
  checkWaiters.delete(check.id);
  for (const resolve of waiters ?? []) resolve(check);
}

/** Wait for pre-analysis `id` to end; its validation summary when done, its issues otherwise. */
export async function checkDone(id: number): Promise<Record<string, unknown>> {
  const check = await wait<Api.Check>(
    (resolve) => checkWaiters.set(id, [...(checkWaiters.get(id) ?? []), resolve]),
    async () => {
      const check = await api.checkState(id);
      return finished(check.status) ? check : null;
    },
  );
  if (check.status !== "done") throw failure(check.status, check.error);
  return (check.result ?? {}) as Record<string, unknown>;
}

/** An event: `operation` as it is now. */
export function settleOperation(operation: Api.PendingOperation) {
  if (!finished(operation.status)) return;
  const waiters = operationWaiters.get(operation.id);
  operationWaiters.delete(operation.id);
  for (const resolve of waiters ?? []) resolve(operation);
}

function wait<T>(register: (resolve: (value: T) => void) => void, poll: () => Promise<T | null>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let done = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const settle = (value: T) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(value);
    };
    register(settle);
    const tick = () => {
      poll().then(
        (value) => {
          if (value !== null) settle(value);
          else if (!done) timer = setTimeout(tick, POLL_MS);
        },
        (error: unknown) => {
          if (done) return;
          done = true;
          reject(error instanceof Error ? error : new Error(String(error)));
        },
      );
    };
    timer = setTimeout(tick, POLL_MS);
  });
}

/** The error of a task that did not succeed, as an API error. */
function failure(status: Api.RunStatus, error: Api.RunError | null | undefined): ApiError {
  return new ApiError(status === "cancelled" ? 409 : 422, error?.kind ?? status, error?.issues ?? [status]);
}

/** Wait for run `id` of `project` to end; its record when done, an error otherwise. */
export async function runDone(project: string, id: number): Promise<Api.RunRecord> {
  const run = await wait<Api.RunRecord>(
    (resolve) => runWaiters.set(runKey(project, id), [...(runWaiters.get(runKey(project, id)) ?? []), resolve]),
    async () => {
      const run = await api.run(project, id);
      return finished(run.status) ? run : null;
    },
  );
  if (run.status !== "done") throw failure(run.status, run.error);
  return run;
}

/** Wait for operation `id` to end; the operation when done (its project made), an error otherwise. */
export async function operationDone(id: number): Promise<Api.PendingOperation> {
  const operation = await wait<Api.PendingOperation>(
    (resolve) => operationWaiters.set(id, [...(operationWaiters.get(id) ?? []), resolve]),
    async () => {
      const found = (await api.operations()).find((o) => o.id === id);
      if (found === undefined) throw new ApiError(404, "not_found", [`operation ${id} is no longer listed`]);
      return finished(found.status) ? found : null;
    },
  );
  if (operation.status !== "done") throw failure(operation.status, operation.error);
  return operation;
}

/** Wait for operation run `id` of `project` and return its result (typed by the command that queued it). */
export async function runResult<T>(project: string, id: number): Promise<T> {
  const run = await runDone(project, id);
  if (run.result === undefined || run.result === null) throw new ApiError(500, "internal", [`run ${id} ended without a result`]);
  return run.result as unknown as T;
}
