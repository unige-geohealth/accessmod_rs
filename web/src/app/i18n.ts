// User-visible text, from the catalogue `cargo xtask i18n` builds out of the
// AccessMod 5 dictionary (en, fr, es, pt). Keys and vocabulary follow it.
import catalog from "../i18n/catalog.json";
import { createStore, useStore } from "../state/store";

export type Language = keyof typeof catalog.messages;
export type Key = keyof typeof catalog.messages.en;

/** Languages of the catalogue, in its order. */
export const LANGUAGES: readonly Language[] = catalog.languages.filter(
  (l): l is Language => l in catalog.messages,
);

const STORAGE = "accessmod.language";

function initialLanguage(): Language {
  try {
    const saved = window.localStorage.getItem(STORAGE);
    const found = LANGUAGES.find((l) => l === saved);
    if (found) return found;
  } catch {
    // Storage may be unavailable (private window): the default applies.
  }
  const browser = typeof navigator === "undefined" ? "en" : navigator.language.slice(0, 2);
  return LANGUAGES.find((l) => l === browser) ?? "en";
}

/** The language of the interface: a per-viewer preference. */
export const languageStore = createStore<Language, Language>(initialLanguage(), (_, next) => next);

languageStore.subscribe(() => {
  try {
    window.localStorage.setItem(STORAGE, languageStore.getState());
  } catch {
    // Not persisted: the choice holds for this page only.
  }
});

export function useLanguage(): Language {
  return useStore(languageStore, (s) => s);
}

export function t(key: Key): string {
  const messages: Record<string, string> = catalog.messages[languageStore.getState()];
  return messages[key] ?? catalog.messages.en[key];
}

/** Display name of an analysis. */
export function analysisName(name: string): string {
  const key = `analysis.${name}`;
  return key in catalog.messages.en ? t(key as Key) : name;
}

/** Display name of a data class (AccessMod 5 identifier). */
export function className(code: string): string {
  const names: Record<string, string> = catalog.classes[languageStore.getState()];
  return names[code] ?? code;
}

/** Analysis screens and toolbox tools, in AccessMod 5 order. */
export const ANALYSIS_SCREENS = [
  "travel-time",
  "geographic-coverage",
  "referral",
  "zonal-statistics",
  "scaling-up",
  "optimization",
] as const;
export const TOOLBOX_SCREENS = ["merge-land-cover", "adjust-population-distribution"] as const;
