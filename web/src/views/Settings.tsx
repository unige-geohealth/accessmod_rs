import { LANGUAGES, languageStore, t, useLanguage } from "../app/i18n";

const NAMES: Record<string, string> = {
  en: "English",
  fr: "Français",
  es: "Español",
  pt: "Português",
};

export function SettingsScreen() {
  const language = useLanguage();
  return (
    <section>
      <h1>{t("section.settings")}</h1>
      <label className="panel">
        {t("settings.language")}
        <select
          value={language}
          onChange={(e) => {
            const next = LANGUAGES.find((l) => l === e.target.value);
            if (next) languageStore.dispatch(next);
          }}
        >
          {LANGUAGES.map((l) => (
            <option key={l} value={l}>
              {NAMES[l] ?? l}
            </option>
          ))}
        </select>
      </label>
    </section>
  );
}
