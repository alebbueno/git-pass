import "./styles.css";
import { listen } from "@tauri-apps/api/event";
import logoLightUrl from "./assets/branding/RepoID-logo-fundo-claro.svg";
import logoDarkUrl from "./assets/branding/RepoID-logo-fundo-escuro.svg";
import { api } from "./lib/api";
import { esc, showToast } from "./lib/dom";
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
        <button type="button" class="sidebar-footer" id="daemon-status" title="Estado do gitpassd (abre Definições)">
          <span class="status-dot" aria-hidden="true"></span><span class="status-text">A testar daemon…</span>
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

const ctx: ViewContext = {
  navigate,
  notifyDataChanged: () => document.dispatchEvent(new CustomEvent(DATA_CHANGED_EVENT)),
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
  api.openExternal(a.href).catch((e) => showToast(String(e), "err"));
});

async function refreshSidebar(): Promise<void> {
  try {
    const [accounts, bindings] = await Promise.all([api.accounts(), api.bindings()]);
    setCount("repos", bindings.length);
    setCount("accounts", accounts.length);
    sidebarProfiles.innerHTML = accounts.length
      ? accounts.map((a) => `<div class="sidebar-profile">${profileChip(a)}</div>`).join("")
      : `<p class="muted small">Nenhum perfil ligado.</p>`;
  } catch (e) {
    sidebarProfiles.innerHTML = `<p class="err small">${esc(String(e))}</p>`;
  }
}

function setCount(view: ViewId, n: number): void {
  const el = document.querySelector<HTMLElement>(`[data-count="${view}"]`);
  if (el) el.textContent = n > 0 ? String(n) : "";
}

async function refreshDaemon(): Promise<void> {
  const text = daemonBtn.querySelector<HTMLElement>(".status-text")!;
  try {
    await api.pingDaemon();
    daemonBtn.dataset.state = "ok";
    text.textContent = "gitpassd ativo";
  } catch {
    daemonBtn.dataset.state = "off";
    text.textContent = "gitpassd parado · só hooks";
  }
}

document.addEventListener(DATA_CHANGED_EVENT, () => void refreshSidebar());
void listen("gitpass-config-changed", () => ctx.notifyDataChanged());

const initial = location.hash.replace("#", "") as ViewId;
navigate(VIEWS.some((v) => v.id === initial) ? initial : "overview");
void refreshSidebar();
void refreshDaemon();
window.setInterval(() => void refreshDaemon(), 30_000);
