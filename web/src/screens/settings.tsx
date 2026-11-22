// Settings (AM5 `amManageSettings`): the theme (the system's, light or
// dark), the language and analysis preferences (overwriting outputs), kept
// in this browser, and the version of the server. Administrators see the
// Project Creator download cache, with its size, and can clear it (AM5
// admin tools: cache); AM5 restart and upload limit have no counterpart.
import { AmButton } from "@accessmod/ui/am/messages";
import { Columns } from "@accessmod/ui/am/columns";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { CheckboxField, RadioField, SelectField } from "@accessmod/ui/am/form";
import { SectionCard } from "@accessmod/ui/am/section";
import { type Lang, languageNames, languages, T, useI18n } from "@accessmod/ui/i18n";
import { type Theme, useTheme } from "@accessmod/ui/theme";
import { useState } from "react";
import { api } from "../api/client";
import { keys, queryClient, useCreatorCache, useHealth, useSession } from "../api/queries";
import { useBatch } from "../state/batch";
import { setPreference, usePreference } from "../state/preferences";
import { byteSize } from "../state/size";
import { BatchStatus, EntryList, useFmt } from "./common";

const THEMES: readonly Theme[] = ["system", "light", "dark"];

function LanguagePanel() {
  const { lang, setLang } = useI18n();
  return (
    <SelectField
      label="project_language_select"
      options={languages.map((l) => ({ value: l, label: languageNames[l] }))}
      value={lang}
      onChange={(l) => setLang(l as Lang)}
    />
  );
}

/** The Project Creator download cache: size, and clearing it (administrators). */
function CreatorCachePanel() {
  const f = useFmt();
  const usage = useCreatorCache();
  const [confirming, setConfirming] = useState(false);
  const { batch, start } = useBatch();
  const size = usage.data ? byteSize(usage.data.bytes) : "-";
  const clear = () =>
    start([size], (s) => s, async () => {
      await api.command("creator.cache_clear", {});
      await queryClient.invalidateQueries({ queryKey: keys.creatorCache });
    });
  return (
    <div className="flex flex-col gap-[15px]">
      <p>
        <T k="settings.creator_cache_text" />
      </p>
      <EntryList entries={[[f("settings.creator_cache_usage"), usage.data ? f("settings.creator_cache_size", size, usage.data.files) : "-"]]} />
      <AmButton
        variant="danger"
        block
        icon="delete"
        k="settings.creator_cache_clear"
        disabled={!usage.data || usage.data.files === 0 || batch.state === "running"}
        onClick={() => setConfirming(true)}
      />
      <BatchStatus batch={batch} doneKey="settings.creator_cache_done" />
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} confirm="settings.creator_cache_clear" onConfirm={() => void clear()}>
        {f("settings.creator_cache_confirm", size)}
      </ConfirmDialog>
    </div>
  );
}

export function SettingsScreen() {
  const { t } = useI18n();
  const { theme, setTheme } = useTheme();
  const health = useHealth();
  const admin = useSession().data?.admin === true;
  const overwrite = usePreference("overwrite");
  const advanced = usePreference("advanced");
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
            <CheckboxField label="settings_advanced_tools" checked={advanced} onChange={(on) => setPreference("advanced", on)} />
          </SectionCard>
          <SectionCard title="settings_system_info_title">
            <EntryList entries={[[t("menu_version_current"), health.data?.version ?? "-"]]} />
          </SectionCard>
          {admin && (
            <SectionCard title="settings.creator_cache">
              <CreatorCachePanel />
            </SectionCard>
          )}
        </>
      }
      right={null}
    />
  );
}
