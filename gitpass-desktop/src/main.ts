import "./styles.css";
import { listen } from "@tauri-apps/api/event";
import logoLightUrl from "./assets/branding/RepoID-logo-fundo-claro.svg";
import logoDarkUrl from "./assets/branding/RepoID-logo-fundo-escuro.svg";
import { api } from "./lib/api";
import { integrationApi, integrationHeadline, onIntegrationChanged, type IntegrationStatus } from "./lib/api-integration";
import { errMessage, esc, showToast } from "./lib/dom";
import { brandSymbol, icon, type IconName } from "./lib/icons";
import { profileChip } from "./lib/profile";
import { initTheme } from "./lib/theme";
import { DATA_CHANGED_EVENT, type MountView, type ViewContext, type ViewId } from "./lib/view";
import { mount as mountOverview } from "./views/overview";
import { mount as mountRepos } from "./views/repos";
import { mount as mountAccounts } from "./views/accounts";
import { mount as mountCloud } from "./views/cloud";
import { mount as mountSettings } from "./views/settings";

initTheme();

const VIEWS: { id: ViewId; label: string; icon: IconName; mount: MountView }[] = [
  { id: "overview", label: "Painel", icon: "dashboard", mount: mountOverview },
  { id: "repos", label: "Repositórios", icon: "repo", mount: mountRepos },
  { id: "accounts", label: "Contas", icon: "user", mount: mountAccounts },
  { id: "cloud", label: "Na cloud", icon: "cloud", mount: mountCloud },
  { id: "settings", label: "Definições", icon: "settings", mount: mountSettings },
];

const app = document.querySelector<HTMLDivElement>("#app")!;

app.innerHTML = `
  <div class="app-canvas-bg">
    <div class="app-layout">
      <aside class="sidebar" aria-label="Navegação">
        <div class="brand">
          <div class="brand-mark" aria-hidden="true">${brandSymbol()}</div>
          <div class="brand-text">
            <img class="brand-wordmark wordmark-for-light" src="${logoLightUrl}" alt="RepoID" width="140" height="22" />
            <img class="brand-wordmark wordmark-for-dark" src="${logoDarkUrl}" alt="RepoID" width="140" height="22" />
            <div class="brand-sub">A conta certa em cada projeto.</div>
          </div>
        </div>
        <nav class="nav" aria-label="Secções">
          ${VIEWS.map(
            (v) => `<button type="button" class="nav-item" data-view="${v.id}">${icon(v.icon, 18)}<span>${esc(v.label)}</span><span class="nav-count" data-count="${v.id}"></span></button>`
          ).join("")}
        </nav>
        <section class="sidebar-profiles" aria-label="Perfis">
          <h2 class="eyebrow">Perfis</h2>
          <div id="sidebar-profiles" class="sidebar-profiles-list"></div>
        </section>
        <button type="button" class="sidebar-footer" id="daemon-status" title="Integração com o Git (abre Definições)">
          <span class="status-dot" aria-hidden="true"></span><span class="status-text">A verificar integração…</span>
        </button>
      </aside>
      <main class="main" id="view-root" tabindex="-1"></main>
    </div>
  </div>
`;

const viewRoot = document.getElementById("view-root")!;
const daemonBtn = document.getElementById("daemon-status") as HTMLButtonElement;
const sidebarProfiles = document.getElementById("sidebar-profiles")!;

let current: ViewId | null = null;
let cleanup: (() => void) | void;

/**
 * Uma mutação costuma avisar duas vezes (a vista chama `notifyDataChanged` e o backend emite
 * `gitpass-config-changed`); juntamos os avisos próximos para as vistas não recarregarem em duplicado.
 */
let dataChangedTimer: number | undefined;
function notifyDataChanged(): void {
  window.clearTimeout(dataChangedTimer);
  dataChangedTimer = window.setTimeout(() => document.dispatchEvent(new CustomEvent(DATA_CHANGED_EVENT)), 120);
}

const ctx: ViewContext = {
  navigate,
  notifyDataChanged,
};

function navigate(view: ViewId): void {
  if (current === view) return;
  const def = VIEWS.find((v) => v.id === view) ?? VIEWS[0];
  if (typeof cleanup === "function") cleanup();
  current = def.id;
  for (const btn of document.querySelectorAll<HTMLButtonElement>(".nav-item")) {
    const active = btn.dataset.view === def.id;
    btn.classList.toggle("active", active);
    if (active) btn.setAttribute("aria-current", "page");
    else btn.removeAttribute("aria-current");
  }
  viewRoot.innerHTML = "";
  viewRoot.scrollTop = 0;
  location.hash = def.id;
  cleanup = def.mount(viewRoot, ctx);
}

for (const btn of document.querySelectorAll<HTMLButtonElement>(".nav-item")) {
  btn.addEventListener("click", () => navigate(btn.dataset.view as ViewId));
}
daemonBtn.addEventListener("click", () => navigate("settings"));

/* O WebView não abre target="_blank"; links externos vão para o browser do sistema. */
document.addEventListener("click", (ev) => {
  const a = (ev.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
  if (!a || !/^https?:\/\//.test(a.href)) return;
  ev.preventDefault();
  api.openExternal(a.href).catch((e) => showToast(errMessage(e), "err"));
});

let sidebarSeq = 0;
async function refreshSidebar(): Promise<void> {
  const seq = ++sidebarSeq;
  try {
    const [accounts, bindings] = await Promise.all([api.accounts(), api.bindings()]);
    if (seq !== sidebarSeq) return;
    setCount("repos", bindings.length);
    setCount("accounts", accounts.length);
    sidebarProfiles.innerHTML = accounts.length
      ? accounts.map((a) => `<div class="sidebar-profile">${profileChip(a)}</div>`).join("")
      : `<p class="muted small">Nenhum perfil ligado.</p>`;
  } catch (e) {
    if (seq !== sidebarSeq) return;
    sidebarProfiles.innerHTML = `<p class="err small">${esc(errMessage(e))}</p>`;
  }
}

function setCount(view: ViewId, n: number): void {
  const el = document.querySelector<HTMLElement>(`[data-count="${view}"]`);
  if (el) el.textContent = n > 0 ? String(n) : "";
}

function renderIntegration(st: IntegrationStatus): void {
  const h = integrationHeadline(st);
  daemonBtn.dataset.state = h.kind === "success" ? "ok" : "off";
  daemonBtn.querySelector<HTMLElement>(".status-text")!.textContent = `Integração Git · ${h.label}`;
  daemonBtn.title = h.detail;
}

async function refreshDaemon(): Promise<void> {
  try {
    renderIntegration(await integrationApi.status());
  } catch {
    daemonBtn.dataset.state = "off";
    daemonBtn.querySelector<HTMLElement>(".status-text")!.textContent = "Integração Git · indisponível";
  }
}

document.addEventListener(DATA_CHANGED_EVENT, () => void refreshSidebar());
void listen("gitpass-config-changed", () => ctx.notifyDataChanged()).catch(() => {});
void onIntegrationChanged(renderIntegration);

const initial = location.hash.replace("#", "") as ViewId;
navigate(VIEWS.some((v) => v.id === initial) ? initial : "overview");
void refreshSidebar();
void refreshDaemon();
window.setInterval(() => void refreshDaemon(), 30_000);
/* A instalação automática da integração corre no arranque e pode terminar antes de o WebView ouvir
   `gitpass-integration-changed`; estas verificações extra evitam mostrar «Reparar» durante 30 s. */
for (const ms of [3_000, 8_000]) window.setTimeout(() => void refreshDaemon(), ms);
