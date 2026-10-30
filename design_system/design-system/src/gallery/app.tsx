import { Fragment, useEffect, useState } from "react";
import who from "@/assets/who.svg";
import { AppShell } from "@/components/am/app-shell";
import { languageNames, languages, type Lang, useI18n } from "@/i18n";
import { type Theme, useTheme } from "@/lib/theme";
import { routes, sectionRoute } from "./routes";

const useHash = () => {
  const read = () => location.hash.replace(/^#\/?/, "") || "analysis-accessibility";
  const [hash, setHash] = useState(read);
  useEffect(() => {
    const on = () => setHash(read());
    addEventListener("hashchange", on);
    return () => removeEventListener("hashchange", on);
  }, []);
  return [hash, (h: string) => (location.hash = `/${h}`)] as const;
};

/**
 * Gallery: each route rebuilds one AM5 screen from the design system.
 * The floating panel switches route, theme, language, and overlays the AM5
 * screenshot (references/screens/<id>/<lang>.png) to compare at 1440px.
 * Query `?chrome=0` hides the panel (visual tests).
 */
export function GalleryApp() {
  const [id, go] = useHash();
  const route = routes.find((r) => r.id === id) ?? routes[0];
  const showPanel = new URLSearchParams(location.search).get("chrome") !== "0";

  return (
    <>
      <AppShell
        logo={<img src={who} alt="World Health Organization" className="max-h-10 w-full px-[15px]" />}
        project="DEMO"
        version="Version 5.9.2-beta.6"
        active={route.section}
        onNavigate={(s) => go(sectionRoute[s] ?? s)}
      >
        <Fragment key={route.id}>{route.render()}</Fragment>
      </AppShell>
      {showPanel && <Panel id={route.id} noReference={route.noReference} go={go} />}
    </>
  );
}

function Panel({ id, noReference, go }: { id: string; noReference?: boolean; go: (h: string) => void }) {
  const { theme, setTheme } = useTheme();
  const { lang, setLang, t } = useI18n();
  const [overlay, setOverlay] = useState(0);
  const [blend, setBlend] = useState<"normal" | "difference">("normal");
  const [open, setOpen] = useState(true);
  const select = "h-7 border border-input bg-input-background px-1 text-sm";

  return (
    <>
      {overlay > 0 && !noReference && (
        <img
          src={`/screens/${id}/${lang}.png`}
          alt=""
          className="pointer-events-none absolute top-0 left-0 z-40 w-[1440px] max-w-none"
          style={{ opacity: overlay, mixBlendMode: blend }}
        />
      )}
      <div className="fixed right-3 bottom-3 z-50 flex flex-col gap-2 border border-border bg-popover p-2 text-sm shadow-lg">
        <button type="button" className="self-end text-xs text-muted-foreground" onClick={() => setOpen(!open)}>
          {open ? "hide ▾" : "gallery ▴"}
        </button>
        {open && (
          <div className="grid grid-cols-[auto_1fr] items-center gap-x-2 gap-y-1.5">
            <span>Screen</span>
            <select className={select} value={id} onChange={(e) => go(e.target.value)}>
              {routes.map((r) => (
                <option key={r.id}>{r.id}</option>
              ))}
            </select>
            <span>{t("theme.title")}</span>
            <select className={select} value={theme} onChange={(e) => setTheme(e.target.value as Theme)}>
              {(["light", "dark", "system"] as const).map((v) => (
                <option key={v} value={v}>
                  {t(`theme.${v}`)}
                </option>
              ))}
            </select>
            <span>{t("project_language_title")}</span>
            <select className={select} value={lang} onChange={(e) => setLang(e.target.value as Lang)}>
              {languages.map((l) => (
                <option key={l} value={l}>
                  {languageNames[l]}
                </option>
              ))}
            </select>
            {!noReference && (
              <>
                <span>AM5 overlay</span>
                <input type="range" min={0} max={1} step={0.05} value={overlay} onChange={(e) => setOverlay(Number(e.target.value))} />
                <span>Blend</span>
                <select className={select} value={blend} onChange={(e) => setBlend(e.target.value as typeof blend)}>
                  <option value="normal">normal</option>
                  <option value="difference">difference</option>
                </select>
                <span />
                <a className="text-link hover:underline" href={`/screens/${id}/${lang}.png`} target="_blank" rel="noreferrer">
                  open AM5 reference
                </a>
              </>
            )}
          </div>
        )}
      </div>
    </>
  );
}
