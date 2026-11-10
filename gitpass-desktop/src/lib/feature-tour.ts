/**
 * Tour pelos recursos (separado do guia de primeira configuração em `onboarding.ts`).
 */
export type FeatureTourState = "done" | "skipped";

const KEY = "gitpass.featureTour";

/** Abre o tour no shell (`installFeatureTour`). */
export const FEATURE_TOUR_REQUEST_EVENT = "gitpass:show-feature-tour";

/** Tour concluído ou saltado (atualizar teasers na UI). */
export const FEATURE_TOUR_CHANGED_EVENT = "gitpass:feature-tour-changed";

export type FeatureTourRequestDetail = {
  /** Só abre se o utilizador ainda não concluiu nem saltou o tour. */
  auto?: boolean;
};

export function getFeatureTour(): FeatureTourState | null {
  try {
    const v = window.localStorage.getItem(KEY);
    return v === "done" || v === "skipped" ? v : null;
  } catch {
    return null;
  }
}

export function setFeatureTour(state: FeatureTourState): void {
  try {
    window.localStorage.setItem(KEY, state);
  } catch {
    /* ignorar */
  }
}

export function requestFeatureTour(detail: FeatureTourRequestDetail = {}): void {
  if (detail.auto && getFeatureTour() !== null) return;
  document.dispatchEvent(new CustomEvent<FeatureTourRequestDetail>(FEATURE_TOUR_REQUEST_EVENT, { detail }));
}
