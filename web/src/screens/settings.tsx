// Settings (AM5 `amManageSettings`): the theme (the system's, light or
// dark), the language and analysis preferences (overwriting outputs), kept
// in this browser, and the version of the server. AM5 admin tools (restart, cache, upload limit) have no
// counterpart: the server needs none of them.
import { Columns } from "@accessmod/ui/am/columns";
import { CheckboxField, RadioField } from "@accessmod/ui/am/form";
import { SectionCard } from "@accessmod/ui/am/section";
import { useI18n } from "@accessmod/ui/i18n";
import { type Theme, useTheme } from "@accessmod/ui/theme";
import { useHealth } from "../api/queries";
import { setPreference, usePreference } from "../state/preferences";
import { EntryList } from "./common";
import { LanguagePanel } from "./project";

const THEMES: readonly Theme[] = ["system", "light", "dark"];

export function SettingsScreen() {
  const { t } = useI18n();
  const { theme, setTheme } = useTheme();
  const health = useHealth();
  const overwrite = usePreference("overwrite");
  return (
    <Columns
      left={
        <>
          <SectionCard title="main_ui_tab_settings" icon="settings">
            <RadioField
              label="theme.title"
              value={theme}
              onChange={(v) => setTheme(THEMES.find((x) => x === v) ?? "system")}
              options={THEMES.map((value) => ({ value, label: `theme.${value}` as const }))}
            />
            <LanguagePanel />
          </SectionCard>
          <SectionCard title="settings.analyses">
            <CheckboxField label="settings.overwrite" checked={overwrite} onChange={(on) => setPreference("overwrite", on)} />
          </SectionCard>
          <SectionCard title="settings_system_info_title">
            <EntryList entries={[[t("menu_version_current"), health.data?.version ?? "-"]]} />
          </SectionCard>
        </>
      }
      right={null}
    />
  );
}
