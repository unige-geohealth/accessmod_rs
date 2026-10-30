import { I18nProvider } from "@accessmod/ui/i18n";
import { ThemeProvider } from "@accessmod/ui/theme";
import { TooltipProvider } from "@accessmod/ui/ui/tooltip";
import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";
import { api } from "./api/client";
import { keys, queryClient } from "./api/queries";
import { App } from "./app/App";
import { formatRoute, HOME } from "./app/route";
import { startProject } from "./state/start";

const root = document.getElementById("root");
if (!root) throw new Error("#root missing");

/**
 * A URL naming no screen opens the default project (else the demo): decided
 * once, before the first render, so no effect moves the user later.
 */
async function openStartProject() {
  if (window.location.hash !== "") return;
  try {
    const [projects, settings] = await Promise.all([
      queryClient.fetchQuery({ queryKey: keys.projects, queryFn: api.projects }),
      queryClient.fetchQuery({ queryKey: keys.settings, queryFn: api.settings }),
    ]);
    const slug = startProject(projects, settings);
    if (slug !== null) window.history.replaceState(null, "", formatRoute({ ...HOME, project: slug }));
  } catch {
    // The home screen; its queries report the error.
  }
}

void openStartProject().then(() =>
  createRoot(root).render(
    <StrictMode>
      <ThemeProvider>
        <I18nProvider>
          <TooltipProvider>
            <QueryClientProvider client={queryClient}>
              <App />
            </QueryClientProvider>
          </TooltipProvider>
        </I18nProvider>
      </ThemeProvider>
    </StrictMode>,
  ),
);
