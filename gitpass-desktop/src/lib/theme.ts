export type ThemeMode = "light" | "dark" | "system";

const KEY = "gitpass-theme";

export function storedTheme(): ThemeMode {
  const v = localStorage.getItem(KEY);
  return v === "light" || v === "dark" ? v : "system";
}

export function applyTheme(mode: ThemeMode): void {
  const root = document.documentElement;
  if (mode === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", mode);
}

export function setTheme(mode: ThemeMode): void {
  localStorage.setItem(KEY, mode);
  applyTheme(mode);
  document.dispatchEvent(new CustomEvent("gitpass:theme-changed", { detail: mode }));
}

export function initTheme(): void {
  applyTheme(storedTheme());
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (storedTheme() === "system") applyTheme("system");
  });
}
