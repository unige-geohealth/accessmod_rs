// A small typed store: state changes only through `update(state, message)`,
// a pure function. Views read with selectors; nothing writes state in
// reaction to other state (docs/ui.md, "State").
import { useSyncExternalStore } from "react";

export interface Store<S, M> {
  getState(): S;
  dispatch(message: M): void;
  subscribe(listener: () => void): () => void;
}

export function createStore<S, M>(initial: S, update: (state: S, message: M) => S): Store<S, M> {
  let state = initial;
  const listeners = new Set<() => void>();
  return {
    getState: () => state,
    dispatch(message) {
      const next = update(state, message);
      if (next !== state) {
        state = next;
        for (const listener of listeners) listener();
      }
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

/** Read `select(state)`; the selector must return a value or an existing
 * part of the state, never a new object. */
export function useStore<S, M, T>(store: Store<S, M>, select: (state: S) => T): T {
  return useSyncExternalStore(store.subscribe, () => select(store.getState()));
}

/** Undo and redo over immutable states. */
export interface History<S> {
  past: S[];
  present: S;
  future: S[];
}

export type HistoryMessage<M> = { type: "undo" } | { type: "redo" } | { type: "do"; message: M };

const MAX_HISTORY = 100;

export function withHistory<S, M>(
  update: (state: S, message: M) => S,
): (history: History<S>, message: HistoryMessage<M>) => History<S> {
  return (history, message) => {
    switch (message.type) {
      case "undo": {
        const previous = history.past.at(-1);
        if (previous === undefined) return history;
        return {
          past: history.past.slice(0, -1),
          present: previous,
          future: [history.present, ...history.future],
        };
      }
      case "redo": {
        const [next, ...rest] = history.future;
        if (next === undefined) return history;
        return { past: [...history.past, history.present], present: next, future: rest };
      }
      case "do": {
        const present = update(history.present, message.message);
        if (present === history.present) return history;
        return {
          past: [...history.past, history.present].slice(-MAX_HISTORY),
          present,
          future: [],
        };
      }
    }
  };
}

export function history<S>(present: S): History<S> {
  return { past: [], present, future: [] };
}
