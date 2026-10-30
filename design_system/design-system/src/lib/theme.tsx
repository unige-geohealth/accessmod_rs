import { createContext, type ReactNode, useCallback, useContext, useEffect, useState } from "react";

export type Theme = "light" | "dark" | "system";
type ThemeState = { theme: Theme; setTheme: (t: Theme) => void };

const ThemeContext = createContext<ThemeState | null>(null);

const dark = (t: Theme) => t === "dark" || (t === "system" && matchMedia("(prefers-color-scheme: dark)").matches);

/** Light is the AM5 look; dark is the only visual addition. Class `dark` on <html>. */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(() => (localStorage.getItem("am.theme") as Theme) ?? "light");

  useEffect(() => {
    const apply = () => document.documentElement.classList.toggle("dark", dark(theme));
    apply();
    const mq = matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [theme]);

  const setTheme = useCallback((t: Theme) => {
    localStorage.setItem("am.theme", t);
    setThemeState(t);
  }, []);

  return <ThemeContext value={{ theme, setTheme }}>{children}</ThemeContext>;
}

export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme outside ThemeProvider");
  return ctx;
}
