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

/** The run a screen started, and whether its outcome was acknowledged. */
export interface Session {
  run: number | null;
  acknowledged: boolean;
}

export type SessionMessage = { type: "started"; run: number } | { type: "acknowledged" };

export const updateSession = (session: Session, message: SessionMessage): Session =>
  message.type === "started" ? { run: message.run, acknowledged: false } : { ...session, acknowledged: true };

export const sessionStore = (project: string, analysis: string): Store<Session, SessionMessage> =>
  keyed(`session/${project}/${analysis}`, { run: null, acknowledged: true }, updateSession);

/** A validation message: a catalogue key, or a key with its `%s` arguments. */
export type Issue = Key | { key: Key; args: (string | number)[] };
