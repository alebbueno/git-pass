/** `repos/map` («Mapear pasta») não aparece no menu: fica dentro de Repositórios. */
export type ViewId = "overview" | "repos" | "repos/map" | "accounts" | "cloud" | "integration" | "diagnostics" | "settings";

/**
 * Rota do hash: uma tela ou `repos/<binding_id>` (gaveta de detalhes de um repositório, id com `encodeURIComponent`).
 * Pode terminar em `?chave=valor&…` (ex.: `repos/<id>?tab=push&branch=main`, `repos?review=1`); a tela recebe isso em `query`.
 */
export type Route = ViewId | `${ViewId}?${string}` | `repos/${string}`;

export type NavigateOptions = {
  /** Troca a rota atual sem criar uma entrada nova no histórico (ex.: trocar de aba). */
  replace?: boolean;
};

export type ViewContext = {
  navigate: (route: Route, opts?: NavigateOptions) => void;
  /** Avisa o shell e as outras telas de que contas/vínculos mudaram. */
  notifyDataChanged: () => void;
  /**
   * A tela montada recebe as mudanças de parâmetro (`repos/<id>` ↔ `repos`) sem ser remontada.
   * O shell esquece o handler ao trocar de tela.
   */
  onRoute: (handler: (param: string | null, query: URLSearchParams) => void) => void;
};

/**
 * Cada tela exporta `mount`. O shell limpa `root` antes de montar.
 * `param` é o resto da rota (`repos/<id>` → `<id>`), já decodificado, ou `null`; `query` é o que vem depois de `?`.
 * Devolve uma função opcional de limpeza (timers, listeners globais, polling).
 */
export type MountView = (root: HTMLElement, ctx: ViewContext, param?: string | null, query?: URLSearchParams) => void | (() => void);

export const DATA_CHANGED_EVENT = "gitpass:data-changed";

/** A gaveta de detalhes abriu (`detail.id`) ou fechou (`detail.id === null`). */
export const DRAWER_EVENT = "gitpass:drawer";
export type DrawerEventDetail = { id: string | null };
