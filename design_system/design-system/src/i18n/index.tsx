import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from "react";
import catalog from "./catalog.json";

export const languages = ["en", "fr", "es", "pt"] as const;
export type Lang = (typeof languages)[number];
/** AM5 dictionary id, e.g. "main_ui_tab_data", or "class.<am5 class>". */
export type Key = keyof (typeof catalog)["en"];

export const languageNames: Record<Lang, string> = {
  en: "English",
  fr: "Français",
  es: "Español",
  pt: "Português",
};

type I18n = { lang: Lang; setLang: (l: Lang) => void; t: (key: Key) => string };

const I18nContext = createContext<I18n | null>(null);

const stored = (): Lang => {
  const l = localStorage.getItem("am.lang");
  return (languages as readonly string[]).includes(l ?? "") ? (l as Lang) : "en";
};

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(stored);
  const setLang = useCallback((l: Lang) => {
    localStorage.setItem("am.lang", l);
    document.documentElement.lang = l;
    setLangState(l);
  }, []);
  const value = useMemo<I18n>(
    () => ({ lang, setLang, t: (key) => catalog[lang][key] ?? catalog.en[key] ?? key }),
    [lang, setLang],
  );
  return <I18nContext value={value}>{children}</I18nContext>;
}

export function useI18n() {
  const ctx = useContext(I18nContext);
  if (!ctx) throw new Error("useI18n outside I18nProvider");
  return ctx;
}

/** Translated text; `data-amt-id` keeps the AM5 key visible in the DOM. */
export function T({ k }: { k: Key }) {
  const { t } = useI18n();
  return <span data-amt-id={k}>{t(k)}</span>;
}
