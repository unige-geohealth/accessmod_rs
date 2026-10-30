import { I18nProvider } from "@accessmod/ui/i18n";
import { ThemeProvider } from "@accessmod/ui/theme";
import { TooltipProvider } from "@accessmod/ui/ui/tooltip";
import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";
import { queryClient } from "./api/queries";
import { App } from "./app/App";

const root = document.getElementById("root");
if (!root) throw new Error("#root missing");

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
);
