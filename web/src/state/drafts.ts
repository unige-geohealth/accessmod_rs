// Drafts of the analysis screens and the run each one started, one store
// per project and analysis, kept while the client runs (not across reloads).
import type { Key } from "@accessmod/ui/i18n";
import { createStore, type Store } from "./store";

const stores = new Map<string, Store<unknown, unknown>>();

function keyed<S, M>(key: string, initial: S, update: (state: S, message: M) => S): Store<S, M> {
  let store = stores.get(key) as Store<S, M> | undefined;
  if (store === undefined) {
    store = createStore(initial, update);
    stores.set(key, store as Store<unknown, unknown>);
  }
  return store;
}

/** The draft store of `analysis` in `project`. */
export const draftStore = <S, M>(project: string, analysis: string, empty: S, update: (state: S, message: M) => S): Store<S, M> =>
  keyed(`draft/${project}/${analysis}`, empty, update);

/** The last run a screen started (zonal statistics shows its table), and
 * whether Compute was pressed and the run is not queued yet (`starting`:
 * one click, one job). */
export interface Session {
  run: number | null;
  starting: boolean;
}

export type SessionMessage = { type: "starting" } | { type: "started"; run: number } | { type: "start_failed" };

export function updateSession(session: Session, message: SessionMessage): Session {
  switch (message.type) {
    case "starting":
      return { ...session, starting: true };
    case "started":
      return { run: message.run, starting: false };
    case "start_failed":
      return { ...session, starting: false };
  }
}

export const sessionStore = (project: string, analysis: string): Store<Session, SessionMessage> =>
  keyed(`session/${project}/${analysis}`, { run: null, starting: false }, updateSession);

/** A validation message: a catalogue key, or a key with its `%s` arguments. */
export type Issue = Key | { key: Key; args: (string | number)[] };
