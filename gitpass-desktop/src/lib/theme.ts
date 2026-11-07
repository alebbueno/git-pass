export type ThemeMode = "light" | "dark" | "system";

const KEY = "gitpass-theme";
const TRANSPARENCY_KEY = "gitpass-reduce-transparency";

export const THEME_CHANGED_EVENT = "gitpass:theme-changed";

const systemDark = (): MediaQueryList => window.matchMedia("(prefers-color-scheme: dark)");

export function storedTheme(): ThemeMode {
  const v = localStorage.getItem(KEY);
  return v === "light" || v === "dark" ? v : "system";
}

/** Tema efetivo agora («Sistema» segue o macOS). */
export function resolvedTheme(mode: ThemeMode = storedTheme()): "light" | "dark" {
  if (mode === "system") return systemDark().matches ? "dark" : "light";
  return mode;
}

export function applyTheme(mode: ThemeMode): void {
  document.documentElement.setAttribute("data-theme", resolvedTheme(mode));
}

export function setTheme(mode: ThemeMode): void {
  localStorage.setItem(KEY, mode);
  applyTheme(mode);
  document.dispatchEvent(new CustomEvent(THEME_CHANGED_EVENT, { detail: mode }));
}

/** O WebView nem sempre expõe `prefers-reduced-transparency`; esta opção troca o vidro por superfícies sólidas. */
export function reducedTransparency(): boolean {
  return localStorage.getItem(TRANSPARENCY_KEY) === "1";
}

export function setReducedTransparency(on: boolean): void {
  if (on) localStorage.setItem(TRANSPARENCY_KEY, "1");
  else localStorage.removeItem(TRANSPARENCY_KEY);
  applyTransparency();
  document.dispatchEvent(new CustomEvent(THEME_CHANGED_EVENT, { detail: storedTheme() }));
}

function applyTransparency(): void {
  const root = document.documentElement;
  if (reducedTransparency()) root.setAttribute("data-transparency", "reduced");
  else root.removeAttribute("data-transparency");
}

export function initTheme(): void {
  applyTheme(storedTheme());
  applyTransparency();
  systemDark().addEventListener("change", () => {
    if (storedTheme() !== "system") return;
    applyTheme("system");
    document.dispatchEvent(new CustomEvent(THEME_CHANGED_EVENT, { detail: "system" }));
  });
}
