import { type ReactNode, useState } from "react";
import { type Key, T, useI18n } from "#ui/i18n";
import { cn } from "#ui/lib/utils";
import { type IconName, icons } from "./icons";

export type NavItem = { id: string; label: Key; icon: IconName };

/** The seven AM5 sections, in AM5 order (ui.R › sidebarMenu). */
export const sections: NavItem[] = [
  { id: "project", label: "main_ui_tab_project", icon: "project" },
  { id: "data", label: "main_ui_tab_data", icon: "data" },
  { id: "analysis", label: "main_ui_tab_analysis", icon: "analysis" },
  { id: "toolbox", label: "main_ui_tab_toolbox", icon: "toolbox" },
  { id: "logs", label: "main_ui_tab_logs", icon: "logs" },
  { id: "settings", label: "main_ui_tab_settings", icon: "settings" },
  { id: "about", label: "main_ui_tab_about", icon: "about" },
];

type AppShellProps = {
  logo: ReactNode;
  /** Product name above the current project (AM5: "AccessMod 5"). */
  product?: string;
  project: string;
  version: string;
  active: string;
  onNavigate: (id: string) => void;
  /** Right side of the header bar, after the menu toggle. */
  headerExtra?: ReactNode;
  children: ReactNode;
};

/**
 * AM5 layout: black sidebar (230px) with the WHO logo on top of it, a white
 * header bar with the menu toggle, and the content area (padding 20px).
 */
export function AppShell({ logo, product = "AccessMod 5", project, version, active, onNavigate, headerExtra, children }: AppShellProps) {
  const [open, setOpen] = useState(true);
  const { t } = useI18n();
  const MenuIcon = icons.menu;

  return (
    <div
      className={cn(
        "grid min-h-svh grid-rows-[var(--header-height)_1fr] bg-background text-foreground transition-[grid-template-columns]",
        open ? "grid-cols-[var(--sidebar-width)_1fr]" : "grid-cols-[0_1fr]",
      )}
    >
      <a
        href="https://www.who.int"
        className="row-start-1 flex items-center justify-center overflow-hidden border-r border-sidebar-border bg-sidebar text-sidebar-foreground"
      >
        {logo}
      </a>

      <header className="col-start-2 row-start-1 flex items-stretch border-b border-header-border bg-header">
        <button
          type="button"
          aria-label={t("nav.toggle")}
          onClick={() => setOpen(!open)}
          className="flex w-[var(--header-height)] items-center justify-center border-r border-header-border text-header-foreground hover:bg-accent"
        >
          <MenuIcon className="size-4" />
        </button>
        <div className="flex flex-1 items-center justify-end gap-2 px-4">{headerExtra}</div>
      </header>

      <aside className="row-start-2 overflow-hidden bg-sidebar text-sidebar-foreground">
        <Sidebar product={product} project={project} version={version} active={active} onNavigate={onNavigate} />
      </aside>

      <main className="col-start-2 row-start-2 min-w-0 p-[var(--content-padding)]">{children}</main>
    </div>
  );
}

function Sidebar({ product, project, version, active, onNavigate }: Omit<AppShellProps, "logo" | "children" | "headerExtra">) {
  return (
    <div className="w-[var(--sidebar-width)]">
      <hr className="my-5 border-sidebar-border" />
      <div className="px-1 text-center">
        <h4 className="mt-2.5 mb-2.5 text-lg">{product}</h4>
        <span className="font-semibold">
          <T k="main_ui_current_project" />
        </span>
        <h4 className="mt-2.5 mb-2.5 text-lg uppercase">{project}</h4>
      </div>
      <hr className="my-5 border-sidebar-border" />
      <nav>
        <ul>
          {sections.map((s) => {
            const Icon = icons[s.icon];
            const on = s.id === active;
            return (
              <li key={s.id}>
                <button
                  type="button"
                  onClick={() => onNavigate(s.id)}
                  aria-current={on ? "page" : undefined}
                  className={cn(
                    "flex w-full items-center gap-1.5 border-l-3 py-3 pr-4 pl-[15px] text-left",
                    on
                      ? "border-sidebar-primary bg-sidebar-accent text-sidebar-accent-foreground"
                      : "border-transparent text-sidebar-muted hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                  )}
                >
                  <Icon className="size-4 shrink-0" />
                  <T k={s.label} />
                </button>
              </li>
            );
          })}
        </ul>
      </nav>
      <hr className="my-5 border-sidebar-border" />
      <div className="p-5 opacity-90">{version}</div>
    </div>
  );
}
