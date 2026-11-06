// Preferences of this browser (Settings), as theme and language: on/off
// switches in localStorage (`am.<name>`), read through
// `useSyncExternalStore`, shared by every screen and kept in step across
// tabs.
import { useSyncExternalStore } from "react";

export type Preference =
  /** Analyses may replace datasets of their output classes and tags (AM5 "overwrite warning"). */
  | "overwrite"
  /** Options AM5 shows only to advanced users ("Enable advanced options in modules"). */
  | "advanced";

const storageKey = (p: Preference) => `am.${p}`;
const listeners = new Set<() => void>();
/** Choices storage refused (private mode): they last until reload. */
const unsaved = new Map<Preference, boolean>();

export function readPreference(p: Preference): boolean {
  const kept = unsaved.get(p);
  if (kept !== undefined) return kept;
  try {
    return localStorage.getItem(storageKey(p)) === "1";
  } catch {
    return false;
  }
}

export function setPreference(p: Preference, on: boolean) {
  try {
    if (on) localStorage.setItem(storageKey(p), "1");
    else localStorage.removeItem(storageKey(p));
    unsaved.delete(p);
  } catch {
    unsaved.set(p, on);
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  window.addEventListener("storage", listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", listener);
  };
}

export const usePreference = (p: Preference): boolean =>
  useSyncExternalStore(
    subscribe,
    () => readPreference(p),
    () => false,
  );
