export type ViewId = "overview" | "repos" | "accounts" | "cloud" | "settings";

export type ViewContext = {
  navigate: (view: ViewId) => void;
  /** Avisa o shell e as outras telas de que contas/vínculos mudaram. */
  notifyDataChanged: () => void;
};

/**
 * Cada tela exporta `mount`. O shell limpa `root` antes de montar.
 * Devolve uma função opcional de limpeza (timers, listeners globais, polling).
 */
export type MountView = (root: HTMLElement, ctx: ViewContext) => void | (() => void);

export const DATA_CHANGED_EVENT = "gitpass:data-changed";
