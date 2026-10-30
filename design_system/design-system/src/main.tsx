import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/index.css";
import { TooltipProvider } from "#ui/components/ui/tooltip";
import { GalleryApp } from "./gallery/app";
import { I18nProvider } from "./i18n";
import { ThemeProvider } from "./lib/theme";

const root = document.getElementById("root");
if (!root) throw new Error("#root missing");

createRoot(root).render(
  <StrictMode>
    <ThemeProvider>
      <I18nProvider>
        <TooltipProvider>
          <GalleryApp />
        </TooltipProvider>
      </I18nProvider>
    </ThemeProvider>
  </StrictMode>,
);
