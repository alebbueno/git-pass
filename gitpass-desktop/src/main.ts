import "./styles.css";
import { listen } from "@tauri-apps/api/event";
import { api, type AccountRow } from "./lib/api";
import { integrationApi, integrationHeadline, onIntegrationChanged, type IntegrationStatus } from "./lib/api-integration";
import { reposApi } from "./lib/api-repos";
import { systemApi, type Diagnostics } from "./lib/api-system";
import { errMessage, esc, installCopyButtons, showToast } from "./lib/dom";
import { brandSymbol, icon, type IconName } from "./lib/icons";
import { closePopover } from "./lib/popover";
import { profileAvatar, profileName } from "./lib/profile";
import { DIAGNOSTICS_UPDATED_EVENT, initSystemState } from "./lib/system-state";
import { initTheme, resolvedTheme, setTheme, THEME_CHANGED_EVENT } from "./lib/theme";
import { DATA_CHANGED_EVENT, type MountView, type NavigateOptions, type Route, type ViewContext, type ViewId } from "./lib/view";
import { mountCurrentRepoChip } from "./shell/current-repo";
import { mountEmergencyBanner } from "./shell/emergency-banner";
import { createRepoDrawer } from "./views/repo-detail";
import { mountHome } from "./views/welcome";
import { mount as mountRepos } from "./views/repos";
import { mount as mountReposMap } from "./views/repos-map";
import { mount as mountAccounts } from "./views/accounts";
import { mount as mountCloud } from "./views/cloud";
import { mount as mountIntegration } from "./views/integration";
import { mount as mountDiagnostics } from "./views/diagnostics";
import { mount as mountSettings } from "./views/settings";

initTheme();

const OVERLAY_TITLEBAR = import.meta.env.VITE_GITPASS_OVERLAY_TITLEBAR === "1";
if (OVERLAY_TITLEBAR) document.documentElement.classList.add("gp-titlebar-overlay");

/**
 * `parent`: telas fora do menu, que destacam o item do menu indicado e entram nas migalhas.
 * `params`: a tela também atende `<id>/<parâmetro>` (rotas exatas, como `repos/map`, têm prioridade).
 */
type ViewDef = { id: ViewId; label: string; icon: IconName; mount: MountView; parent?: ViewId; params?: boolean };
const VIEWS: ViewDef[] = [
  { id: "overview", label: "Painel", icon: "dashboard", mount: mountHome },
  { id: "repos", label: "Repositórios", icon: "repo", mount: mountRepos, params: true },
  { id: "repos/map", label: "Mapear pasta", icon: "folder", mount: mountReposMap, parent: "repos" },
  { id: "accounts", label: "Contas", icon: "user", mount: mountAccounts },
  { id: "cloud", label: "Na nuvem", icon: "cloud", mount: mountCloud },
  { id: "integration", label: "Integração", icon: "plug", mount: mountIntegration },
  { id: "diagnostics", label: "Diagnóstico", icon: "stethoscope", mount: mountDiagnostics },
  { id: "settings", label: "Configurações", icon: "settings", mount: mountSettings },
];
const NAV_VIEWS = VIEWS.filter((v) => !v.parent);
const viewDef = (id: ViewId): ViewDef => VIEWS.find((v) => v.id === id)!;

const navItem = (v: ViewDef): string =>
  `<button type="button" class="nav-item" data-view="${v.id}" title="${esc(v.label)}" aria-label="${esc(v.label)}">${icon(v.icon)}<span>${esc(
    v.label
  )}</span><span class="nav-count" data-count="${v.id}" aria-hidden="true"></span></button>`;

const drag = OVERLAY_TITLEBAR ? " data-tauri-drag-region" : "";
const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <div class="app" id="gitpass">
    <aside class="rail" aria-label="Barra lateral"${drag}>
      <div class="brand">${brandSymbol()}<div class="brand-copy"><div class="brand-name">GitPass</div><p class="brand-note">Identidades Git</p></div></div>
      <p class="eyebrow" id="rail-nav-title">Área de trabalho</p>
      <nav class="nav" aria-labelledby="rail-nav-title">${NAV_VIEWS.map(navItem).join("")}</nav>
      <section class="rail-profiles" aria-labelledby="rail-profiles-title">
        <h2 class="eyebrow" id="rail-profiles-title">Perfis conectados</h2>
        <div id="rail-profiles"></div>
      </section>
      <div class="rail-foot">
        <button type="button" class="rail-integration" id="rail-integration" data-state="off" title="Integração com o Git">
          <span class="integration-icon">${icon("shield")}</span>
          <span class="rail-integration-text"><strong>Verificando…</strong><small>Git local</small></span>
          ${icon("chevronRight")}
        </button>
      </div>
    </aside>
    <div class="workspace" id="workspace">
      <header class="topbar"${drag}>
        <nav class="crumbs" aria-label="Você está em"><ol id="crumbs"></ol></nav>
        <div class="top-actions">
          <div class="top-current" id="gp-current"></div>
          <button type="button" class="icon-button ghost" id="theme-toggle"></button>
        </div>
      </header>
      <div class="gp-emergency" id="gp-emergency" role="region" aria-label="Modo emergência" hidden></div>
      <main class="screen" id="screen" tabindex="-1"><div class="screen-inner" id="view-root"></div></main>
    </div>
    <aside class="drawer" id="repo-drawer" aria-labelledby="drawer-title" inert></aside>
  </div>
`;

const screen = document.getElementById("screen")!;
const crumbs = document.getElementById("crumbs")!;
const railIntegration = document.getElementById("rail-integration") as HTMLButtonElement;
const railProfiles = document.getElementById("rail-profiles")!;
const themeToggle = document.getElementById("theme-toggle") as HTMLButtonElement;

let viewRoot = document.getElementById("view-root")!;
let current: ViewId | null = null;
let currentRoute: string | null = null;
/** Rota da tela montada por baixo da gaveta (para onde voltar ao fechá-la). */
let baseRoute: string | null = null;
let cleanup: (() => void) | void;
let routeHandler: ((param: string | null, query: URLSearchParams) => void) | null = null;
let accounts: AccountRow[] = [];

/**
 * Uma mutação costuma avisar duas vezes (a tela chama `notifyDataChanged` e o backend emite
 * `gitpass-config-changed`); juntamos os avisos próximos para as telas não recarregarem em duplicado.
 */
let dataChangedTimer: number | undefined;
function notifyDataChanged(): void {
  window.clearTimeout(dataChangedTimer);
  dataChangedTimer = window.setTimeout(() => document.dispatchEvent(new CustomEvent(DATA_CHANGED_EVENT)), 120);
}

const ctx: ViewContext = {
  navigate,
  notifyDataChanged,
  onRoute: (handler) => {
    routeHandler = handler;
  },
};

const drawer = createRepoDrawer(document.getElementById("repo-drawer")!, ctx, {
  baseLabel: () => (current ? viewDef(current).label : "Repositórios"),
  requestClose: () => navigate((baseRoute ?? "repos") as Route, { replace: true }),
});

function resolveRoute(route: string): { def: ViewDef; param: string | null } {
  const exact = VIEWS.find((v) => v.id === route);
  if (exact) return { def: exact, param: null };
  const sub = VIEWS.find((v) => v.params && route.startsWith(`${v.id}/`) && route.length > v.id.length + 1);
  if (sub) {
    const raw = route.slice(sub.id.length + 1);
    try {
      return { def: sub, param: decodeURIComponent(raw) };
    } catch {
      return { def: sub, param: null };
    }
  }
  return { def: VIEWS[0], param: null };
}

function renderCrumbs(def: ViewDef): void {
  const parts: string[] = [`<li><span>Área de trabalho</span></li>`];
  if (def.parent) {
    const p = viewDef(def.parent);
    parts.push(`<li>${icon("chevronRight")}<button type="button" data-crumb="${p.id}">${esc(p.label)}</button></li>`);
  }
  parts.push(`<li>${icon("chevronRight")}<strong aria-current="page">${esc(def.label)}</strong></li>`);
  crumbs.innerHTML = parts.join("");
}

function navigate(route: Route, opts: NavigateOptions = {}): void {
  if (currentRoute === route) return;
  const q = route.indexOf("?");
  const query = new URLSearchParams(q < 0 ? "" : route.slice(q + 1));
  const { def, param } = resolveRoute(q < 0 ? route : route.slice(0, q));
  const search = query.toString();
  const path = `${param === null ? def.id : `${def.id}/${encodeURIComponent(param)}`}${search ? `?${search}` : ""}`;
  currentRoute = path;
  if (location.hash.slice(1) !== path) {
    if (opts.replace) history.replaceState(history.state, "", `#${path}`);
    else location.hash = path;
  }

  // `repos/<id>`: a gaveta abre por cima da tela atual (ou de Repositórios, se nada estiver montado).
  if (def.id === "repos" && param !== null) {
    if (current === null) mountView(def, new URLSearchParams(), def.id);
    drawer.open(param, query);
    return;
  }
  const wasOpen = drawer.isOpen();
  if (wasOpen) drawer.close();
  if (current === def.id && path === baseRoute) return;
  if (current === def.id && routeHandler) {
    baseRoute = path;
    routeHandler(param, query);
    return;
  }
  mountView(def, query, path);
  if (!wasOpen && document.activeElement?.closest(".rail")) screen.focus({ preventScroll: true });
}

function mountView(def: ViewDef, query: URLSearchParams, path: string): void {
  closePopover();
  if (typeof cleanup === "function") cleanup();
  routeHandler = null;
  current = def.id;
  baseRoute = path;
  for (const btn of document.querySelectorAll<HTMLButtonElement>(".nav-item")) {
    if (btn.dataset.view === (def.parent ?? def.id)) btn.setAttribute("aria-current", "page");
    else btn.removeAttribute("aria-current");
  }
  renderCrumbs(def);
  const fresh = document.createElement("div");
  fresh.className = "screen-inner";
  fresh.id = "view-root";
  fresh.dataset.view = def.id;
  viewRoot.replaceWith(fresh);
  viewRoot = fresh;
  screen.scrollTop = 0;
  cleanup = def.mount(viewRoot, ctx, null, query);
}

window.addEventListener("hashchange", () => navigate((location.hash.slice(1) || "overview") as Route));

for (const btn of document.querySelectorAll<HTMLButtonElement>(".nav-item")) {
  btn.addEventListener("click", () => navigate(btn.dataset.view as ViewId));
}
railIntegration.addEventListener("click", () => navigate("integration"));
crumbs.addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-crumb]");
  if (b) navigate(b.dataset.crumb as ViewId);
});
railProfiles.addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-profile]");
  if (b?.dataset.profile) navigate(`repos?profile=${encodeURIComponent(b.dataset.profile)}`);
});

/* O WebView não abre target="_blank"; links externos vão para o navegador do sistema. */
document.addEventListener("click", (ev) => {
  const a = (ev.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
  if (!a || !/^https?:\/\//.test(a.href)) return;
  ev.preventDefault();
  api.openExternal(a.href).catch((e) => showToast(errMessage(e), "err"));
});

installCopyButtons(icon("check", 12));

/* ⌘K (Ctrl K) leva à busca de Repositórios. */
document.addEventListener("keydown", (e) => {
  if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "k" || e.altKey || e.shiftKey) return;
  if (document.querySelector(".modal-backdrop")) return;
  e.preventDefault();
  closePopover();
  if (current !== "repos" || drawer.isOpen()) navigate("repos");
  let tries = 0;
  const focusSearch = (): void => {
    const input = document.getElementById("repo-search") as HTMLInputElement | null;
    if (input) {
      input.focus();
      input.select();
    } else if (++tries < 40) window.setTimeout(focusSearch, 50);
  };
  focusSearch();
});

// ---------- Tema ----------

function renderThemeToggle(): void {
  const dark = resolvedTheme() === "dark";
  const label = dark ? "Usar tema claro" : "Usar tema escuro";
  themeToggle.innerHTML = icon(dark ? "sun" : "moon");
  themeToggle.setAttribute("aria-label", label);
  themeToggle.title = label;
}
themeToggle.addEventListener("click", () => setTheme(resolvedTheme() === "dark" ? "light" : "dark"));
document.addEventListener(THEME_CHANGED_EVENT, renderThemeToggle);
renderThemeToggle();

// ---------- Barra lateral: perfis, contadores e integração ----------

function setCount(view: ViewId, n: number, tone: "muted" | "warning" | "error", srText: string): void {
  const el = document.querySelector<HTMLElement>(`[data-count="${view}"]`);
  if (!el) return;
  el.dataset.tone = tone;
  el.textContent = n > 0 ? n.toLocaleString("pt-BR") : "";
  const item = el.closest<HTMLElement>(".nav-item");
  if (!item) return;
  const label = viewDef(view).label;
  item.setAttribute("aria-label", n > 0 && srText ? `${label}, ${srText}` : label);
  item.title = n > 0 && srText ? `${label} · ${srText}` : label;
}

let sidebarSeq = 0;
async function refreshSidebar(): Promise<void> {
  const seq = ++sidebarSeq;
  try {
    accounts = await api.accounts();
    if (seq !== sidebarSeq) return;
    const disconnected = accounts.filter((a) => !a.has_token).length;
    const n = accounts.length;
    setCount(
      "accounts",
      n,
      disconnected ? "warning" : "muted",
      `${n} ${n === 1 ? "conta" : "contas"}${disconnected ? `, ${disconnected} ${disconnected === 1 ? "desconectada" : "desconectadas"}` : ""}`
    );
    railProfiles.innerHTML = accounts.length
      ? accounts
          .map(
            (a) =>
              `<button type="button" class="rail-profile" data-profile="${esc(a.id)}" title="${esc(`Ver projetos de ${profileName(a)}`)}">${profileAvatar(
                a,
                22
              )}<span class="truncate">${esc(profileName(a))}</span></button>`
          )
          .join("")
      : `<p class="rail-empty">Nenhum perfil conectado.</p>`;
    currentChip.refresh();
  } catch (e) {
    if (seq !== sidebarSeq) return;
    railProfiles.innerHTML = `<p class="rail-empty err">${esc(errMessage(e))}</p>`;
  }
  // A lista de repositórios lê o Git de cada pasta: não segura os perfis.
  try {
    const rows = await reposApi.list();
    if (seq !== sidebarSeq) return;
    setCount("repos", rows.length, "muted", `${rows.length} ${rows.length === 1 ? "repositório" : "repositórios"}`);
  } catch {
    /* o contador some; a tela Repositórios mostra o erro */
  }
}

function renderDiagnosticsCount(d: Diagnostics): void {
  const errors = d.items.filter((i) => i.state === "error").length;
  const warnings = d.items.filter((i) => i.state === "warning").length;
  if (errors > 0) setCount("diagnostics", errors, "error", errors === 1 ? "1 item com erro" : `${errors} itens com erro`);
  else setCount("diagnostics", warnings, "warning", warnings === 1 ? "1 item pede atenção" : `${warnings} itens pedem atenção`);
}

async function refreshDiagnosticsCount(): Promise<void> {
  try {
    renderDiagnosticsCount(await systemApi.diagnostics({ probeTokens: false }));
  } catch {
    setCount("diagnostics", 0, "muted", "");
  }
}

function setIntegration(state: "ok" | "warn" | "err" | "off", title: string, detail: string): void {
  railIntegration.dataset.state = state;
  railIntegration.querySelector("strong")!.textContent = title;
  railIntegration.title = `${detail}. Abrir Integração.`;
  railIntegration.setAttribute("aria-label", `${title}. ${detail}. Abrir Integração`);
}

function renderIntegration(st: IntegrationStatus): void {
  const h = integrationHeadline(st);
  if (!st.enabled) setIntegration("off", "Integração desligada", h.detail);
  else if (h.kind === "success") setIntegration("ok", "Integração ativa", h.detail);
  else setIntegration(h.kind === "error" ? "err" : "warn", "Integração com problema", h.detail);
}

async function refreshDaemon(): Promise<void> {
  try {
    renderIntegration(await integrationApi.status());
  } catch {
    setIntegration("off", "Integração indisponível", "Não foi possível ler o estado da integração");
  }
}

const currentChip = mountCurrentRepoChip(document.getElementById("gp-current")!, ctx, () => accounts);
mountEmergencyBanner(document.getElementById("gp-emergency")!, ctx);
initSystemState();

document.addEventListener(DATA_CHANGED_EVENT, () => void refreshSidebar());
document.addEventListener(DIAGNOSTICS_UPDATED_EVENT, (e) => renderDiagnosticsCount((e as CustomEvent<Diagnostics>).detail));
void listen("gitpass-config-changed", () => ctx.notifyDataChanged()).catch(() => {});
void onIntegrationChanged(renderIntegration);

navigate((location.hash.slice(1) || "overview") as Route);
void refreshSidebar();
void refreshDaemon();
window.setTimeout(() => void refreshDiagnosticsCount(), 2_000);
window.setInterval(() => void refreshDaemon(), 30_000);
window.setInterval(() => void refreshDiagnosticsCount(), 5 * 60_000);
/* A instalação automática da integração corre no arranque e pode terminar antes de o WebView ouvir
   `gitpass-integration-changed`; estas verificações extra evitam mostrar «Reparar» durante 30 s. */
for (const ms of [3_000, 8_000]) window.setTimeout(() => void refreshDaemon(), ms);
