/**
 * Estado do primeiro arranque, guardado em `localStorage`.
 *  - `progress`: a conta já foi ligada mas o guia ainda não terminou (voltar ao Painel retoma o guia).
 *  - `done` / `skipped`: o guia não volta a aparecer sozinho.
 */
export type OnboardingState = "progress" | "done" | "skipped";

const KEY = "gitpass.onboarding";

/** Disparado no `document` para voltar a abrir o guia de boas-vindas (ex.: botão no Painel). */
export const WELCOME_REQUEST_EVENT = "gitpass:show-welcome";

export function getOnboarding(): OnboardingState | null {
  try {
    const v = window.localStorage.getItem(KEY);
    return v === "progress" || v === "done" || v === "skipped" ? v : null;
  } catch {
    return null;
  }
}

export function setOnboarding(state: OnboardingState): void {
  try {
    window.localStorage.setItem(KEY, state);
  } catch {
    /* armazenamento indisponível: o guia volta a aparecer no próximo arranque */
  }
}
