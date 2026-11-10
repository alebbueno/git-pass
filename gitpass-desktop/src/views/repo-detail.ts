import "./repo-detail.css";
import { api, type AccountRow } from "../lib/api";
import { reposApi, type IdentityDiagnosticRow, type RepoRow } from "../lib/api-repos";
import type { CurrentRepoState } from "../lib/api-system";
import { errMessage, esc, relativeTime, shortenPath, showToast, withBusy } from "../lib/dom";
import { codeChip, errorView } from "../lib/errors";
import { icon, iconPaths, type IconName } from "../lib/icons";
import { profileAvatar, profileFamily, profileInitials, profileName } from "../lib/profile";
import { onSystemState, pinRepo, verdictMeta } from "../lib/system-state";
import { resolvedTheme, setTheme, THEME_CHANGED_EVENT } from "../lib/theme";
import { DATA_CHANGED_EVENT, DRAWER_EVENT, type DrawerEventDetail, type ViewContext } from "../lib/view";
import { sourceTag, verdictPill } from "../shell/current-repo";
import { defaultBranchOf, forgetBranches, mountBranchesTab, refreshBranches } from "./repo-branches";
import { openMergeFlow } from "./repo-merge";
import {
  forgetSession,
  isPushing,
  mountPushTab,
  onSessionChange,
  openPushSession,
  outgoingShas,
  pushBranch,
  refreshOutgoing,
  setPushBranch,
} from "./repo-push";
import { closeMenus, identityText, openAccountMenu, openMenu, openModal, plural, profileFor, repoName, whoText } from "./repos-ui";

/** `git log --date=short` devolve `AAAA-MM-DD`; mostra "Hoje", "Ontem" ou "6 de nov.". */
function commitDay(date: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date.trim());
  if (!m) return date;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const days = Math.round((today.getTime() - d.getTime()) / 86_400_000);
  if (days === 0) return "Hoje";
  if (days === 1) return "Ontem";
  const opts: Intl.DateTimeFormatOptions = { day: "numeric", month: "short" };
  if (d.getFullYear() !== today.getFullYear()) opts.year = "numeric";
  return d.toLocaleDateString("pt-BR", opts);
}

export type Tab = "overview" | "push" | "branches" | "history" | "config";

const TABS: { id: Tab; label: string; icon: IconName }[] = [
  { id: "overview", label: "Visão geral", icon: "repo" },
  { id: "push", label: "Enviar", icon: "send" },
  { id: "branches", label: "Branches", icon: "branch" },
  { id: "history", label: "Histórico", icon: "history" },
  { id: "config", label: "Configuração", icon: "settings" },
];

export function isTab(v: string | null): v is Tab {
  return TABS.some((t) => t.id === v);
}

/** O que a rota diz sobre a gaveta: aba e (na aba Enviar) o branch. Campos ausentes mantêm o que já está aberto. */
type DrawerLocation = { tab?: Tab; branch?: string | null };

/** `?tab=push&branch=main` → aba e branch da aba Enviar. */
function locationOf(query: URLSearchParams): DrawerLocation {
  const tab = query.get("tab");
  if (!isTab(tab)) return { tab: "overview" };
  return tab === "push" ? { tab, branch: query.get("branch") || null } : { tab };
}

/** Rota da gaveta; a Visão geral é a aba padrão e não aparece na rota. */
export function drawerRoute(id: string, tab: Tab = "overview", branch: string | null = null): `repos/${string}` {
  const q = new URLSearchParams();
  if (tab !== "overview") q.set("tab", tab);
  if (tab === "push" && branch) q.set("branch", branch);
  const search = q.toString();
  return `repos/${encodeURIComponent(id)}${search ? `?${search}` : ""}`;
}

type HistoryEntry = {
  hash: string;
  subject: string;
  author: string;
  date: string;
  identity_mismatch?: boolean;
  beyond_upstream?: boolean;
};

/** `git log` devolve `%h — %s — %an — %ad`; o assunto pode conter « — ». */
function parseLogLine(line: string): HistoryEntry {
  const parts = line.split(" — ");
  if (parts.length >= 4 && /^[0-9a-f]{4,40}$/i.test(parts[0])) {
    const date = parts.pop()!;
    const author = parts.pop()!;
    const hash = parts.shift()!;
    return { hash, subject: parts.join(" — "), author, date };
  }
  const m = /^([0-9a-f]{7,40})\b\s*(.*)$/i.exec(line.trim());
  return m ? { hash: m[1], subject: m[2], author: "", date: "" } : { hash: "", subject: line, author: "", date: "" };
}

/** Nome curto do projeto (o repositório no GitHub ou a pasta). */
function shortName(b: RepoRow): string {
  if (b.github_repo) return b.github_repo;
  const parts = b.root_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? b.root_path;
}

function branchText(b: RepoRow): string {
  return b.branch ?? (b.detached_at ? `fora de um branch (${b.detached_at})` : "sem branch");
}

/** Arte Projeto → Conta → Branch, na cor do perfil (`currentColor` = `--signal`). */
function routeArt(initials: string): string {
  const path = "M74 152h58c42 0 41-46 86-46h85c39 0 44-43 86-43h41";
  return `<svg class="route-art" viewBox="0 0 520 220" aria-hidden="true" focusable="false">
    <defs>
      <radialGradient id="ra-aura"><stop class="ra-aura-0"/><stop class="ra-aura-1" offset="1"/></radialGradient>
      <linearGradient id="ra-ink"><stop class="ra-ink-0"/><stop class="ra-ink-1" offset=".53"/><stop class="ra-ink-2" offset="1"/></linearGradient>
      <linearGradient id="ra-glass" x1="0" y1="0" x2=".8" y2="1"><stop class="ra-glass-0"/><stop class="ra-glass-1" offset=".51"/><stop class="ra-glass-2" offset="1"/></linearGradient>
      <linearGradient id="ra-facet" x1="0" y1="0" x2=".85" y2="1"><stop class="ra-facet-0"/><stop class="ra-facet-1" offset=".56"/><stop class="ra-facet-2" offset="1"/></linearGradient>
    </defs>
    <ellipse cx="257" cy="109" rx="210" ry="101" fill="url(#ra-aura)"/>
    <g class="ra-orbits" fill="none" stroke-opacity=".19">
      <ellipse cx="263" cy="110" rx="211" ry="60" transform="rotate(-17 263 110)"/>
      <ellipse cx="263" cy="110" rx="224" ry="73" transform="rotate(-17 263 110)" stroke-dasharray="2 7"/>
      <path d="M17 169 503 43M91 211l361-99" stroke-opacity=".14"/>
    </g>
    <g class="ra-stars" opacity=".55">
      <circle cx="29" cy="95" r="1.2"/><circle cx="151" cy="55" r="1.2"/><circle cx="169" cy="177" r="1.5"/>
      <circle cx="333" cy="33" r="1.5"/><circle cx="470" cy="130" r="1.3"/><circle cx="390" cy="151" r="1.4"/>
    </g>
    <path d="${path}" fill="none" stroke="url(#ra-ink)" stroke-width="8" stroke-opacity=".13"/>
    <path d="${path}" fill="none" stroke="url(#ra-ink)" stroke-width="1.55" stroke-opacity=".79"/>
    <path class="signal-streak ra-streak" d="${path}" pathLength="1" fill="none" stroke-width="3" stroke-linecap="round"/>
    <circle class="ra-dot" cx="134" cy="152" r="3"/><circle cx="359" cy="79" r="3" fill="currentColor"/>
    <g transform="translate(74 152)">
      <rect class="ra-node-shadow" x="-26" y="-23" width="54" height="54" rx="15" stroke-opacity=".3"/>
      <rect class="ra-node-edge" x="-27" y="-27" width="54" height="54" rx="15" fill="url(#ra-glass)" stroke-opacity=".83"/>
      <path class="ra-node-shine" d="M-17-22h29q9 0 10 9" fill="none" stroke-opacity=".44"/>
      <svg class="ra-node-icon" x="-13" y="-13" width="26" height="26" viewBox="0 0 24 24" fill="none" stroke-width="1.55" stroke-linecap="round" stroke-linejoin="round">${iconPaths(
        "repo"
      )}</svg>
      <text class="art-label" text-anchor="middle" x="0" y="46">PROJETO</text>
    </g>
    <g transform="translate(257 106)">
      <circle r="54" fill="none" stroke="currentColor" stroke-opacity=".25" stroke-dasharray="2 5"/>
      <circle r="46" fill="currentColor" fill-opacity=".05" stroke="currentColor" stroke-opacity=".31"/>
      <rect class="ra-hub-shadow" x="-35" y="-29" width="72" height="72" rx="22" stroke="currentColor" stroke-opacity=".37"/>
      <rect x="-36" y="-36" width="72" height="72" rx="22" fill="url(#ra-glass)" stroke="currentColor"/>
      <rect x="-35" y="-35" width="70" height="70" rx="21" fill="url(#ra-facet)"/>
      <path class="ra-hub-shine" d="M-23-29h34q16 0 18 15M-23 30h34" fill="none" stroke-opacity=".53"/>
      <text class="art-initials" text-anchor="middle" x="0" y="7">${esc(initials)}</text>
      <text class="art-label" text-anchor="middle" x="0" y="72">CONTA</text>
    </g>
    <g transform="translate(430 63)">
      <rect class="ra-branch-shadow" x="-26" y="-23" width="54" height="54" rx="15" stroke-opacity=".3"/>
      <rect class="ra-branch-edge" x="-27" y="-27" width="54" height="54" rx="15" fill="url(#ra-glass)" stroke-opacity=".85"/>
      <path class="ra-branch-shine" d="M-17-22h29q9 0 10 9" fill="none" stroke-opacity=".47"/>
      <svg class="ra-branch-icon" x="-12" y="-12" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">${iconPaths(
        "branch"
      )}</svg>
      <text class="art-label" text-anchor="middle" x="0" y="46">BRANCH</text>
    </g>
  </svg>`;
}

export type DrawerHooks = {
  /** Nome da tela por baixo («‹ Repositórios»). */
  baseLabel: () => string;
  /** O usuário pediu para fechar (Esc, ✕, «Concluir»): o shell volta à rota da tela. */
  requestClose: () => void;
};

export type DrawerHandle = {
  open: (id: string, query: URLSearchParams) => void;
  close: () => void;
  isOpen: () => boolean;
};

/** Gaveta de detalhes do repositório: desliza da direita sobre a área de trabalho (que fica `inert`). */
export function createRepoDrawer(el: HTMLElement, ctx: ViewContext, hooks: DrawerHooks): DrawerHandle {
  const appEl = el.closest<HTMLElement>(".app")!;
  const workspace = appEl.querySelector<HTMLElement>(".workspace")!;
  el.innerHTML = `
    <header class="drawer-top" data-ref="top"></header>
    <div class="drawer-body" data-ref="scroll">
      <div class="drawer-inner">
        <div class="repo-hero" data-ref="hero"></div>
        <div class="drawer-alerts" data-ref="alerts" hidden></div>
        <div class="tabs" data-ref="tabs" role="tablist" aria-label="Informações do repositório"></div>
        <div class="drawer-panel" data-ref="panel" id="drawer-panel" role="tabpanel" tabindex="-1"></div>
      </div>
    </div>
    <footer class="drawer-footer" data-ref="footer"></footer>`;
  const ref = <T extends HTMLElement = HTMLElement>(name: string): T => el.querySelector<T>(`[data-ref="${name}"]`)!;
  const topEl = ref("top");
  const scrollEl = ref("scroll");
  const heroEl = ref("hero");
  const alertsEl = ref("alerts");
  const tabsEl = ref("tabs");
  const panel = ref("panel");
  const footerEl = ref("footer");

  let openId: string | null = null;
  let repo: RepoRow | null = null;
  let repos: RepoRow[] = [];
  let accounts: AccountRow[] = [];
  let status: "idle" | "loading" | "ok" | "error" = "idle";
  let loadError = "";
  let loadSeq = 0;
  let reloadTimer: number | undefined;
  let tab: Tab = "overview";
  let editing = false;
  let tabHandle: { refresh: () => void; dispose: () => void } | null = null;
  let history: { id: string; status: "loading" | "ok" | "error"; entries: HistoryEntry[]; error: string } | null = null;
  let identityDiag: {
    id: string;
    status: "loading" | "ok" | "error";
    value?: IdentityDiagnosticRow;
    error?: string;
    showOrigins?: boolean;
    remote?: string | null;
    seq: number;
  } | null = null;
  let identityDiagSeq = 0;
  let system: CurrentRepoState | null = null;
  let restoreFocus: HTMLElement | null = null;

  const isOpen = (): boolean => openId !== null;
  const current = (): RepoRow => repo!;
  const who = (id: string): string => whoText(accounts, id);
  const accountOf = (b: RepoRow): AccountRow | undefined => accounts.find((a) => a.id === b.account_id);
  const showsIdentityDiagnostic = (): boolean => tab === "overview" || tab === "config";

  function teardownTab(): void {
    tabHandle?.dispose();
    tabHandle = null;
  }

  // ---------- Abrir e fechar ----------

  function open(id: string, query: URLSearchParams): void {
    const loc = locationOf(query);
    const wasOpen = isOpen();
    const changed = id !== openId;
    if (!wasOpen) {
      const active = document.activeElement as HTMLElement | null;
      restoreFocus = active && active !== document.body ? active : null;
    }
    if (changed) {
      teardownTab();
      closeMenus();
      openId = id;
      repo = repos.find((r) => r.id === id) ?? null;
      tab = loc.tab ?? "overview";
      editing = false;
      history = null;
      identityDiag = null;
      if (tab === "push" && loc.branch !== undefined && repo) setPushBranch(id, normalizeBranch(repo, loc.branch));
      openPushSession(id);
      heroEl.style.removeProperty("--glint-x");
      heroEl.style.removeProperty("--glint-y");
      renderToolbar();
      renderAll();
      scrollEl.scrollTop = 0;
      void load();
    } else if (repo) {
      const wantTab = loc.tab ?? tab;
      if (wantTab === "push" && loc.branch !== undefined && setPushBranch(id, normalizeBranch(repo, loc.branch))) openPushSession(id);
      if (wantTab !== tab) selectTab(wantTab, false, false);
    }
    if (!wasOpen) {
      appEl.setAttribute("data-inspecting", "");
      document.documentElement.classList.add("gp-inspecting");
      el.inert = false;
      workspace.inert = true;
      document.dispatchEvent(new CustomEvent<DrawerEventDetail>(DRAWER_EVENT, { detail: { id } }));
    } else if (changed) {
      document.dispatchEvent(new CustomEvent<DrawerEventDetail>(DRAWER_EVENT, { detail: { id } }));
    }
    if (!wasOpen || changed) el.querySelector<HTMLElement>("#drawer-title")?.focus({ preventScroll: true });
  }

  function close(): void {
    if (!isOpen()) return;
    const id = openId;
    teardownTab();
    closeMenus();
    openId = null;
    editing = false;
    appEl.removeAttribute("data-inspecting");
    document.documentElement.classList.remove("gp-inspecting");
    el.inert = true;
    workspace.inert = false;
    document.dispatchEvent(new CustomEvent<DrawerEventDetail>(DRAWER_EVENT, { detail: { id: null } }));
    const row = id ? workspace.querySelector<HTMLElement>(`[data-repo-id="${CSS.escape(id)}"]`) : null;
    const target = restoreFocus?.isConnected && !el.contains(restoreFocus) ? restoreFocus : row;
    (target ?? workspace.querySelector<HTMLElement>("#screen"))?.focus({ preventScroll: true });
    restoreFocus = null;
  }

  async function load(): Promise<void> {
    const seq = ++loadSeq;
    const id = openId;
    if (!repo) {
      status = "loading";
      renderAll();
    }
    try {
      const [accs, list] = await Promise.all([api.accounts(), reposApi.list()]);
      if (seq !== loadSeq || openId !== id) return;
      accounts = accs;
      repos = list;
      const next = list.find((r) => r.id === id) ?? null;
      const firstShow = !repo && next;
      repo = next;
      status = "ok";
      if (firstShow || !next) {
        renderAll();
        if (firstShow) el.querySelector<HTMLElement>("#drawer-title")?.focus({ preventScroll: true });
      } else renderChrome();
    } catch (e) {
      if (seq !== loadSeq || openId !== id) return;
      if (repo) {
        showToast(`Não foi possível atualizar o repositório: ${errMessage(e)}`, "err");
        return;
      }
      status = "error";
      loadError = errMessage(e);
      renderAll();
    }
  }

  function scheduleReload(): void {
    if (!isOpen()) return;
    window.clearTimeout(reloadTimer);
    reloadTimer = window.setTimeout(() => void load(), 250);
  }

  /** Uma alteração feita aqui: atualiza a gaveta e avisa as telas. */
  function onRepoUpdated(row: RepoRow): void {
    const i = repos.findIndex((r) => r.id === row.id);
    if (i >= 0) repos[i] = row;
    else repos.push(row);
    if (repo?.id === row.id) repo = row;
    renderChrome();
    ctx.notifyDataChanged();
  }

  // ---------- Estrutura ----------

  function renderToolbar(): void {
    const dark = resolvedTheme() === "dark";
    const themeLabel = dark ? "Usar tema claro" : "Usar tema escuro";
    topEl.innerHTML = `<div class="drawer-toolbar">
      <div class="drawer-nav">
        <button type="button" class="control ghost" data-action="close" title="Voltar para ${esc(hooks.baseLabel())}">${icon("back")}<span>${esc(
          hooks.baseLabel()
        )}</span></button>
        ${icon("chevronRight")}<span class="caption muted">Detalhes</span>
      </div>
      <div class="drawer-actions">
        <button type="button" class="icon-button ghost" data-action="theme" aria-label="${themeLabel}" title="${themeLabel}">${icon(dark ? "sun" : "moon")}</button>
        <button type="button" class="control ghost drawer-close" data-action="close" aria-label="Fechar detalhes" title="Fechar detalhes (Esc)"><kbd aria-hidden="true">Esc</kbd>${icon(
          "x"
        )}</button>
      </div>
    </div>`;
  }

  function renderAll(): void {
    teardownTab();
    closeMenus();
    if (!repo) {
      heroEl.hidden = true;
      alertsEl.hidden = true;
      tabsEl.hidden = true;
      footerEl.hidden = true;
      panel.removeAttribute("aria-labelledby");
      if (status === "loading" || status === "idle") {
        panel.innerHTML = `<div class="drawer-loading" aria-busy="true"><span class="sr-only">Carregando o repositório…</span>
          <div class="skeleton drawer-skeleton-hero"></div><div class="skeleton drawer-skeleton-tabs"></div><div class="skeleton drawer-skeleton-card"></div></div>`;
      } else if (status === "error") {
        panel.innerHTML = errorView({
          title: "Não foi possível carregar o repositório",
          error: loadError,
          actions: [{ label: "Tentar de novo", action: "retry-load", icon: "refresh", primary: true }],
        });
      } else {
        panel.innerHTML = `<section class="surface"><div class="empty-state">
          <div class="empty-icon">${icon("search")}</div>
          <h3>Repositório não encontrado</h3>
          <p>Ele pode ter sido desvinculado. Volte para a lista e escolha outro.</p>
          <div class="row-actions"><button type="button" class="control" data-action="close">${icon("back")}<span>Voltar</span></button></div>
        </div></section>`;
      }
      return;
    }
    heroEl.hidden = false;
    tabsEl.hidden = false;
    footerEl.hidden = false;
    renderHero();
    renderAlerts();
    renderTabs();
    renderFooter();
    mountTab();
  }

  /** Atualiza tudo menos o conteúdo da aba (que se atualiza sozinho). */
  function renderChrome(): void {
    if (!repo) {
      renderAll();
      return;
    }
    if (heroEl.hidden) {
      renderAll();
      return;
    }
    renderHero();
    renderAlerts();
    renderTabs();
    renderFooter();
    tabHandle?.refresh();
  }

  function renderHero(): void {
    const b = current();
    const acc = accountOf(b);
    const fam = b.account_missing ? "neutral" : profileFamily(b.account_id);
    const prof = profileFor(accounts, b);
    const name = profileName(prof);
    const ahead = aheadCount(b);
    heroEl.className = `repo-hero ${fam}`;
    heroEl.innerHTML = `
      ${routeArt(b.account_missing ? "?" : profileInitials(name))}
      <div class="hero-copy">
        <p class="hero-label">Repositório local</p>
        <h2 id="drawer-title" tabindex="-1" title="${esc(b.root_path)}">${esc(shortName(b))}</h2>
        <p class="hero-origin">${esc(b.github_owner && b.github_repo ? `${b.github_owner}/${b.github_repo}` : shortenPath(b.root_path))}</p>
        <div class="hero-tags">
          <span class="hero-tag">${icon("branch")}<code class="truncate">${esc(branchText(b))}</code></span>
          <span class="hero-tag">${
            b.account_missing
              ? `${icon("alert")}<span class="truncate">Conta removida</span>`
              : `${profileAvatar(acc ?? prof, 17)}<span class="truncate">${esc(name)}</span>`
          }</span>
          ${ahead > 0 ? `<span class="hero-tag">${icon("arrowUp")}<span>${esc(plural(ahead, "commit para enviar", "commits para enviar"))}</span></span>` : ""}
        </div>
      </div>`;
  }

  function renderAlerts(): void {
    const b = current();
    const acc = accountOf(b);
    const out: string[] = [];
    if (!b.exists) {
      out.push(
        `<div class="callout error" role="alert">${icon("alert")}<span>A pasta não existe mais neste Mac. Se você a moveu, desvincule o repositório e vincule de novo na pasta nova.</span></div>`
      );
    }
    if (b.account_missing) {
      out.push(
        `<div class="callout error" role="alert">${icon("alert")}<span>A conta deste vínculo foi removida. Escolha outra conta em «Trocar conta» para continuar usando o GitPass neste repositório.</span></div>`
      );
    } else if (acc && !acc.has_token) {
      out.push(
        `<div class="callout warning">${icon("alert")}<span>A conta ${esc(who(acc.id))} está desconectada. Conecte de novo para verificar e enviar commits.</span>
          <button type="button" class="control" data-action="go-accounts">${icon("user")}<span>Reconectar conta</span></button></div>`
      );
    }
    if (b.remote_has_credentials) {
      out.push(
        `<div class="callout error" role="alert">${icon("lock")}<span>O endereço do <span class="mono">origin</span> deste repositório tem uma senha ou chave escrita nele. Troque por <span class="mono">${esc(
          b.remote_url ?? ""
        )}</span> — o GitPass cuida do acesso automaticamente.</span></div>`
      );
    }
    alertsEl.hidden = out.length === 0;
    alertsEl.innerHTML = out.join("");
  }

  function aheadCount(b: RepoRow): number {
    const shas = outgoingShas(b.id);
    return shas ? shas.length : b.exists ? (b.sync?.ahead ?? 0) : 0;
  }

  function renderTabs(): void {
    const b = current();
    const ahead = aheadCount(b);
    const focused = tabsEl.contains(document.activeElement);
    tabsEl.style.setProperty("--tab-count", String(TABS.length));
    tabsEl.style.setProperty("--tab-index", String(TABS.findIndex((t) => t.id === tab)));
    tabsEl.innerHTML = TABS.map((t) => {
      const sel = t.id === tab;
      const count = t.id === "push" && ahead > 0 ? `<span class="tab-count">${ahead}<span class="sr-only"> para enviar</span></span>` : "";
      return `<button type="button" role="tab" class="tab" id="drawer-tab-${t.id}" data-tab="${t.id}" aria-selected="${sel}" aria-controls="drawer-panel" tabindex="${
        sel ? 0 : -1
      }">${icon(t.icon)}${esc(t.label)}${count}</button>`;
    }).join("");
    panel.setAttribute("aria-labelledby", `drawer-tab-${tab}`);
    if (focused) tabsEl.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
  }

  function needsFix(b: RepoRow): boolean {
    return b.exists && !b.account_missing && (b.identity.state === "different" || b.identity.state === "not_set");
  }

  function renderFooter(): void {
    const b = current();
    const focusKey = footerEl.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.action : undefined;
    footerEl.innerHTML = `<div class="drawer-footer-inner">
      <div class="left-actions">
        <button type="button" class="control" data-action="finder"${b.exists ? "" : " disabled"}>${icon("folder")}<span>Finder</span></button>
        <button type="button" class="control" data-action="terminal"${b.exists ? "" : " disabled"}>${icon("terminal")}<span>Terminal</span></button>
      </div>
      <div class="right-actions">${
        needsFix(b)
          ? `<button type="button" class="control primary" data-action="fix-identity">${icon("shieldCheck")}<span>Corrigir identidade</span></button>`
          : `<button type="button" class="control ghost" data-action="close">${icon("check")}<span>Concluir</span></button>`
      }</div>
    </div>`;
    if (focusKey) footerEl.querySelector<HTMLElement>(`[data-action="${focusKey}"]`)?.focus();
  }

  // ---------- Abas ----------

  /** O branch atual vira `null` na sessão, para a aba Enviar seguir a pasta. */
  function normalizeBranch(b: RepoRow, branch: string | null): string | null {
    return branch && branch !== b.branch ? branch : null;
  }

  function syncRoute(): void {
    if (!repo || !isOpen()) return;
    ctx.navigate(drawerRoute(repo.id, tab, tab === "push" ? pushBranch(repo.id) : null), { replace: true });
  }

  function selectTab(next: Tab, focus: boolean, route = true): void {
    if (next === tab) {
      if (focus) tabsEl.querySelector<HTMLElement>(`#drawer-tab-${next}`)?.focus();
      if (route) syncRoute();
      return;
    }
    tab = next;
    editing = false;
    renderTabs();
    mountTab();
    if (focus) tabsEl.querySelector<HTMLElement>(`#drawer-tab-${next}`)?.focus();
    if (route) syncRoute();
  }

  /** Abre a aba Enviar para `branch` (ou o atual), validando de novo. */
  function openPushFor(branch: string | null): void {
    const b = current();
    setPushBranch(b.id, normalizeBranch(b, branch));
    openPushSession(b.id);
    if (tab === "push") mountTab();
    selectTab("push", false);
    window.setTimeout(() => (panel.querySelector<HTMLElement>("#push-branch-select") ?? tabsEl.querySelector<HTMLElement>("#drawer-tab-push"))?.focus(), 0);
  }

  function openMerge(source: string, target: string | null, commit: string | null = null): void {
    const b = current();
    openMergeFlow({
      repo: current,
      source,
      target: target ?? defaultBranchOf(b.id)?.name ?? null,
      commit,
      onPushTarget: (t) => {
        if (repo?.id === b.id) openPushFor(t);
      },
      onCheckedOut: (row) => afterCheckout(row),
      onApplied: () => {
        if (repo?.id !== b.id) return;
        history = null;
        void refreshOutgoing(b.id, false);
        if (tab === "history") mountTab();
      },
    });
  }

  function afterCheckout(row: RepoRow): void {
    if (pushBranch(row.id) === row.branch) setPushBranch(row.id, null);
    history = null;
    onRepoUpdated(row);
    void refreshOutgoing(row.id, false);
    void refreshBranches(row.id, false);
    if (tab === "history" && repo?.id === row.id) mountTab();
  }

  function mountTab(): void {
    teardownTab();
    if (!repo) return;
    panel.innerHTML = "";
    panel.classList.remove("drawer-panel");
    void panel.offsetWidth;
    panel.classList.add("drawer-panel");
    if (tab === "overview") {
      tabHandle = mountOverview();
    } else if (tab === "push") {
      const host = document.createElement("div");
      host.className = "drawer-tab-host";
      panel.appendChild(host);
      tabHandle = mountPushTab(host, {
        repo: current,
        accounts: () => accounts,
        goAccounts: () => ctx.navigate("accounts"),
        changeAccount: (btn) => pickAccount(btn),
        fixIdentity: (btn) => void fixIdentity(btn),
        openMerge: (source, target) => openMerge(source, target),
        onBranchChange: () => syncRoute(),
      });
    } else if (tab === "branches") {
      const host = document.createElement("div");
      host.className = "drawer-tab-host";
      panel.appendChild(host);
      tabHandle = mountBranchesTab(host, {
        repo: current,
        openMerge: (source, target) => openMerge(source, target),
        openPush: (branch) => openPushFor(branch),
        onCheckedOut: (row) => afterCheckout(row),
      });
    } else if (tab === "history") {
      tabHandle = mountHistory();
    } else {
      tabHandle = mountConfig();
    }
  }

  // ---------- Visão geral ----------

  function identityPoint(label: string, avatar: string, name: string, email: string): string {
    return `<div class="identity-point">
      <p class="eyebrow">${esc(label)}</p>
      <div class="identity-person">${avatar}<strong>${esc(name)}</strong></div>
      <code>${esc(email)}</code>
    </div>`;
  }

  function identityDiagBlock(b: RepoRow): string {
    const d = identityDiag?.id === b.id ? identityDiag : null;
    if (!d || d.status === "loading") {
      return `<div class="identity-diag identity-diag--busy" aria-busy="true"><span class="skeleton" style="height:52px;border-radius:10px"></span></div>`;
    }
    if (d.status === "error") {
      return `<div class="identity-diag identity-diag--error" role="alert">${icon("alert")}<span>${esc(d.error ?? "Não foi possível carregar o diagnóstico.")}</span>
        <button type="button" class="control sm" data-action="diag-retry">${icon("refresh")}<span>Tentar novamente</span></button></div>`;
    }
    const v = d.value!;
    const originsOpen = d.showOrigins ?? false;
    const remoteOpts =
      v.push.available_remotes.length > 1
        ? `<label class="identity-diag-remote"><span class="sr-only">Remoto para analisar o push</span>
            <select data-action="diag-remote">${v.push.available_remotes
              .map(
                (r) =>
                  `<option value="${esc(r)}"${r === v.push.selected_remote ? " selected" : ""}>${esc(r)}</option>`
              )
              .join("")}</select></label>`
        : "";
    const tokenLine =
      v.push.token_login || v.push.token_message
        ? `<p class="identity-diag-token${v.push.token_push_permission === false ? " is-warn" : ""}">${icon("user")}<span>${
            v.push.token_login ? `@${esc(v.push.token_login)}` : ""
          }${v.push.token_message ? ` · ${esc(v.push.token_message)}` : ""}</span></p>`
        : `<button type="button" class="control ghost sm" data-action="diag-verify-token">${icon("cloud")}<span>Verificar token no GitHub</span></button>`;
    const origins =
      v.commit.origins.length && originsOpen
        ? `<dl class="identity-origins">${v.commit.origins
            .map(
              (o) =>
                `<div><dt class="mono">${esc(o.key)}</dt><dd>${o.value ? esc(o.value) : "<span class='muted'>—</span>"}${
                  o.scope || o.origin
                    ? `<span class="caption muted">${[o.scope, o.origin].filter((x): x is string => Boolean(x)).map(esc).join(" · ")}</span>`
                    : ""
                }</dd></div>`
            )
            .join("")}${v.push.evidence.map((e) => `<div class="identity-evidence"><dt>Push</dt><dd>${esc(e)}</dd></div>`).join("")}</dl>`
        : "";
    const commitTone =
      v.commit.state === "correct" ? "" : v.commit.state === "unavailable" ? " neutral" : " warning";
    const pushTone = v.push.auth_state === "gitpass_routed" ? "" : " neutral";
    return `<div class="identity-diag">
      <div class="identity-diag-row${commitTone}">
        <p class="eyebrow">Commit</p>
        <p class="identity-diag-summary">${esc(v.commit.summary)}</p>
        ${v.commit.detail ? `<p class="caption muted">${esc(v.commit.detail)}</p>` : ""}
      </div>
      <div class="identity-diag-row${pushTone}">
        <p class="eyebrow">Push</p>
        <p class="identity-diag-summary">${esc(v.push.destination_summary)}</p>
        <p class="identity-diag-sub">${esc(v.push.auth_summary)}</p>
        ${remoteOpts}
        ${tokenLine}
      </div>
      <div class="identity-diag-tools">
        <button type="button" class="control ghost sm" data-action="diag-origins" aria-expanded="${originsOpen}">${icon("info")}<span>${originsOpen ? "Ocultar origem" : "Ver origem"}</span></button>
        <button type="button" class="control ghost sm" data-action="diag-retry">${icon("refresh")}<span>Atualizar diagnóstico</span></button>
      </div>
      ${origins}`;
  }

  function identityHtml(b: RepoRow): string {
    const acc = accountOf(b);
    const id = b.identity;
    const ok = b.exists && id.state === "correct";
    const badge = !b.exists
      ? `<span class="state neutral">${icon("info")}Pasta não encontrada</span>`
      : ok
        ? `<span class="state">${icon("check")}Identidade correta</span>`
        : id.state === "different"
          ? `<span class="state warning">${icon("alert")}Identidade divergente</span>`
          : id.state === "not_set"
            ? `<span class="state warning">${icon("alert")}Identidade não definida</span>`
            : `<span class="state neutral">${icon("help")}Sem verificação</span>`;
    const linked = b.account_missing
      ? identityPoint("Conta vinculada", `<span class="avatar neutral" style="--size:26px" aria-hidden="true">?</span>`, "Conta removida", "escolha outra conta")
      : identityPoint(
          "Conta vinculada",
          profileAvatar(acc ?? profileFor(accounts, b), 26),
          profileName(acc ?? profileFor(accounts, b)),
          id.expected_email ?? acc?.email ?? "sem e-mail de commit"
        );
    const localAcc = id.local_email ? accounts.find((a) => a.email && a.email.toLowerCase() === id.local_email!.toLowerCase()) : undefined;
    const localAvatar = localAcc
      ? profileAvatar(localAcc, 26)
      : `<span class="avatar neutral" style="--size:26px" aria-hidden="true">${id.local_name ? esc(profileInitials(id.local_name)) : "?"}</span>`;
    const local = identityPoint(
      "Identidade local",
      localAvatar,
      id.local_name ?? (localAcc ? profileName(localAcc) : "Não definida"),
      id.local_email ?? (b.exists ? "usa a identidade global do Git" : "pasta não encontrada")
    );
    const joinKind = ok ? "" : !b.exists ? "neutral" : "warning";
    const warning = !b.exists
      ? ""
      : id.state === "different"
        ? `<p class="identity-warning">O nome e o e-mail no Git local diferem da conta vinculada.</p>`
        : id.state === "not_set"
          ? `<p class="identity-warning">O Git local não tem nome e e-mail para este projeto: os commits usam a identidade global.</p>`
          : ok && !id.expected_email
            ? `<p class="identity-warning">A conta não tem e-mail de commit definido. Escolha um em Contas.</p>`
            : "";
    const editor = editing
      ? `<form class="profile-editor" data-ref="profile-editor">
          <label class="sr-only" for="drawer-profile">Nova conta vinculada</label>
          <select id="drawer-profile">${accounts
            .map(
              (a) =>
                `<option value="${esc(a.id)}"${a.id === b.account_id ? " selected" : ""}>${esc(
                  a.login && a.label ? `${profileName(a)} (@${a.login})` : profileName(a)
                )}</option>`
            )
            .join("")}</select>
          <button type="submit" class="control primary">Aplicar</button>
          <button type="button" class="control ghost" data-action="cancel-account">${icon("x")}<span>Cancelar</span></button>
        </form>`
      : "";
    const fixBtn =
      b.exists && !b.account_missing && b.identity.state !== "correct"
        ? `<button type="button" class="control primary sm" data-action="fix-identity">${icon("check")}<span>Corrigir identidade</span></button>`
        : "";
    return `<section class="identity" aria-labelledby="drawer-identity-title">
      <div class="identity-head"><h3 id="drawer-identity-title">${icon("shield")}Identidade do Git</h3>${badge}</div>
      ${identityDiagBlock(b)}
      <div class="identity-route">
        ${linked}
        <span class="identity-join ${joinKind}" aria-hidden="true">${icon(ok ? "check" : !b.exists ? "info" : "alert")}</span>
        ${local}
      </div>
      <div class="identity-actions">
        ${warning}
        ${fixBtn}
        <button type="button" class="control" data-action="edit-account" aria-expanded="${editing}"${accounts.length ? "" : " disabled"}>${icon(
          "user"
        )}<span>Trocar conta</span></button>
      </div>
      ${editor}
    </section>`;
  }

  function metaField(label: string, value: string, glyph: IconName, extra = ""): string {
    return `<div class="meta-field">${icon(glyph)}<div><div class="meta-label">${esc(label)}</div><code>${esc(value)}</code></div>${extra}</div>`;
  }

  function syncText(b: RepoRow): string | null {
    if (!b.exists || !b.sync) return null;
    const parts: string[] = [];
    const ahead = aheadCount(b);
    if (ahead > 0) parts.push(plural(ahead, "commit para enviar", "commits para enviar"));
    if (b.sync.behind > 0) parts.push(plural(b.sync.behind, "commit novo no GitHub", "commits novos no GitHub"));
    if (b.sync.dirty) parts.push("alterações sem commit");
    if (!b.sync.has_upstream) parts.push("branch ainda não enviado");
    return parts.length ? parts.join(" · ") : "Em dia com o GitHub";
  }

  function dataHtml(b: RepoRow): string {
    const remote = b.remote_url ? b.remote_url.replace(/^https?:\/\//, "").replace(/\.git$/, "") : null;
    const sync = syncText(b);
    return `<section class="drawer-section" aria-labelledby="drawer-data-title">
      <h3 id="drawer-data-title">${icon("repo")}Dados do repositório</h3>
      <div class="repo-meta">
        ${metaField("Branch atual", branchText(b), "branch")}
        ${metaField(
          "Pasta local",
          b.root_path,
          "folder",
          `<button type="button" class="icon-button ghost" data-action="copy-path" aria-label="Copiar a pasta local" title="Copiar a pasta local">${icon("copy")}</button>`
        )}
        ${
          remote
            ? metaField(
                "Remoto",
                remote,
                "link",
                `<button type="button" class="icon-button ghost" data-action="copy-remote" aria-label="Copiar o endereço remoto" title="Copiar o endereço remoto">${icon(
                  "copy"
                )}</button><a class="icon-button ghost" href="${esc(b.remote_url!)}" aria-label="Abrir no GitHub (abre no navegador)" title="Abrir no GitHub">${icon(
                  "external"
                )}</a>`
              )
            : metaField("Remoto", "origin desconhecido", "link")
        }
        ${sync ? `<div class="meta-field">${icon("activity")}<div><div class="meta-label">Sincronização</div><span class="meta-value">${esc(sync)}</span></div></div>` : ""}
      </div>
    </section>`;
  }

  function usageHtml(b: RepoRow): string {
    const pinned = system?.pinned_binding_id === b.id;
    const cur = system?.current?.binding_id === b.id ? system.current : null;
    const recent = cur ?? system?.recent.find((r) => r.binding_id === b.id) ?? null;
    let verdict: string;
    if (recent) {
      const v = verdictMeta(recent.verdict);
      verdict = `<div class="drawer-verdict drawer-verdict-${v.kind}">
        ${cur ? sourceTag(cur) : `<span class="gp-source">${icon("clock", 12)}<span>Última verificação</span></span>`}
        ${verdictPill(recent.verdict, "md")}
        <span class="drawer-verdict-text">${esc(recent.message || v.detail)}</span>
        ${recent.code ? codeChip(recent.code) : ""}
      </div>`;
    } else {
      verdict = `<p class="caption muted">Quando você usar o Git neste projeto, o último veredito do GitPass aparece aqui.</p>`;
    }
    return `<section class="drawer-section" aria-labelledby="drawer-usage-title">
      <div class="drawer-section-head">
        <h3 id="drawer-usage-title">${icon("target")}Repo atual</h3>
        ${cur ? `<span class="state accent">${icon("check")}É o repo atual</span>` : `<span class="state neutral">Não é o repo atual</span>`}
      </div>
      ${verdict}
      <div class="row-actions drawer-usage-actions">
        <button type="button" class="control${pinned ? " is-pinned" : ""}" data-action="${pinned ? "unpin" : "pin"}" aria-pressed="${pinned}" title="${
          pinned ? "Deixar de usar este repositório como atual" : "Usar este repositório como atual, mesmo com outro app em foco"
        }">${icon(pinned ? "pinOff" : "pin")}<span>${pinned ? "Desafixar" : "Fixar como atual"}</span></button>
        <button type="button" class="control ghost danger" data-action="unbind">${icon("x")}<span>Desvincular…</span></button>
      </div>
    </section>`;
  }

  async function loadIdentityDiagnostic(
    bindingId: string,
    opts: { verifyToken?: boolean; remote?: string | null; showOrigins?: boolean } = {}
  ): Promise<void> {
    const seq = ++identityDiagSeq;
    identityDiag = {
      id: bindingId,
      status: "loading",
      showOrigins: opts.showOrigins ?? identityDiag?.showOrigins,
      remote: opts.remote !== undefined ? opts.remote : identityDiag?.remote,
      seq,
    };
    if (showsIdentityDiagnostic() && repo?.id === bindingId && isOpen()) tabHandle?.refresh();
    try {
      const row = await reposApi.identityDiagnostic(
        bindingId,
        identityDiag.remote ?? null,
        opts.verifyToken ?? false
      );
      if (seq !== identityDiagSeq || openId !== bindingId) return;
      identityDiag = {
        id: bindingId,
        status: "ok",
        value: row,
        showOrigins: opts.showOrigins ?? identityDiag?.showOrigins,
        remote: row.push.selected_remote,
        seq,
      };
    } catch (e) {
      if (seq !== identityDiagSeq || openId !== bindingId) return;
      identityDiag = {
        id: bindingId,
        status: "error",
        error: errMessage(e),
        showOrigins: identityDiag?.showOrigins,
        remote: identityDiag?.remote,
        seq,
      };
    }
    if (showsIdentityDiagnostic() && repo?.id === bindingId && isOpen()) tabHandle?.refresh();
  }

  function mountOverview(): { refresh: () => void; dispose: () => void } {
    const render = (): void => {
      const b = current();
      const focusKey = panel.contains(document.activeElement)
        ? ((document.activeElement as HTMLElement).dataset.action ?? (document.activeElement as HTMLElement).id)
        : undefined;
      panel.innerHTML = `<div class="drawer-stack">
        <div class="repo-layout">${identityHtml(b)}${dataHtml(b)}</div>
        ${usageHtml(b)}
      </div>`;
      if (focusKey) (panel.querySelector<HTMLElement>(`[data-action="${focusKey}"]`) ?? panel.querySelector<HTMLElement>(`#${CSS.escape(focusKey)}`))?.focus();
    };
    const b = current();
    if (b.exists && (!identityDiag || identityDiag.id !== b.id || identityDiag.status === "error")) {
      void loadIdentityDiagnostic(b.id);
    } else if (identityDiag?.id === b.id && identityDiag.status !== "loading") {
      render();
    } else {
      render();
    }
    return { refresh: render, dispose: () => {} };
  }

  // ---------- Histórico ----------

  function mountHistory(): { refresh: () => void; dispose: () => void } {
    const b = current();
    const section = (inner: string, head = ""): string =>
      `<section class="drawer-section"><div class="drawer-section-head"><h3>${icon("history")}Histórico de commits</h3>${head}</div>${inner}</section>`;
    const render = (): void => {
      const r = current();
      if (!r.exists) {
        panel.innerHTML = section(`<p class="caption muted">Histórico indisponível: a pasta não foi encontrada.</p>`);
        return;
      }
      if (!history || history.id !== r.id || history.status === "loading") {
        panel.innerHTML = section(
          `<div class="drawer-history-loading" aria-busy="true"><span class="sr-only">Carregando commits…</span>${Array.from(
            { length: 5 },
            (_, i) => `<div class="skeleton drawer-skeleton-commit" style="width:${92 - (i % 3) * 14}%"></div>`
          ).join("")}</div>`
        );
        return;
      }
      if (history.status === "error") {
        panel.innerHTML = section(
          errorView({
            title: "Não foi possível ler o histórico",
            error: history.error,
            compact: true,
            actions: [{ label: "Tentar de novo", action: "reload-history", icon: "refresh" }],
          })
        );
        panel.querySelector<HTMLButtonElement>('[data-action="reload-history"]')?.addEventListener("click", () => void loadHistory());
        return;
      }
      if (history.entries.length === 0) {
        panel.innerHTML = section(`<p class="caption muted">Ainda não há commits neste repositório.</p>`);
        return;
      }
      const pending = outgoingShas(r.id) ?? [];
      const isPending = (hash: string): boolean => hash.length > 0 && pending.some((s) => s.startsWith(hash.toLowerCase()));
      const nPending = history.entries.filter((e) => isPending(e.hash)).length;
      const head =
        nPending > 0
          ? `<button type="button" class="control sm" data-action="go-push">${icon("send")}<span>Ir para Enviar</span></button>`
          : "";
      panel.innerHTML = section(
        `<p class="caption muted drawer-history-note">${
          nPending > 0
            ? `${esc(plural(nPending, "commit ainda não foi enviado", "commits ainda não foram enviados"))} para o GitHub.`
            : `Últimos ${history.entries.length} commits deste branch.`
        }</p>
        <ul class="commit-list">${history.entries
          .map((e, i) => {
            const p = isPending(e.hash);
            const short = e.hash.slice(0, 7);
            return `<li class="${p ? "pending" : ""}" style="--entry:${Math.min(i, 8) * 45}ms">${icon(p ? "arrowUp" : "commit")}
              <div class="commit-text"><strong>${esc(e.subject)}</strong><span class="commit-meta">${esc(
                [e.author, commitDay(e.date)].filter(Boolean).join(" · ")
              )}${p ? ` · <span class="warn">não enviado</span>` : ""}${
                e.identity_mismatch ? ` · <span class="warn">identidade diferente do perfil</span>` : ""
              }${e.beyond_upstream ? ` · <span class="hint">fora do upstream local</span>` : ""}</span></div>
              ${e.hash ? `<code title="${esc(e.hash)}">${esc(short)}</code>` : ""}
              ${
                e.hash
                  ? `<span class="repos-menu-host"><button type="button" class="icon-button ghost" data-action="commit-menu" data-hash="${esc(
                      e.hash
                    )}" aria-haspopup="menu" aria-expanded="false" aria-label="${esc(`Ações do commit ${short}`)}" title="Ações do commit">${icon(
                      "more"
                    )}</button></span>`
                  : ""
              }
            </li>`;
          })
          .join("")}</ul>`,
        head
      );
      panel.querySelector<HTMLButtonElement>('[data-action="go-push"]')?.addEventListener("click", () => selectTab("push", true));
    };
    const loadHistory = async (): Promise<void> => {
      const id = b.id;
      history = { id, status: "loading", entries: [], error: "" };
      render();
      try {
        const rows = await reposApi.commitReview(id, 40);
        if (history?.id !== id) return;
        history = {
          id,
          status: "ok",
          entries: rows.map((r) => ({
            hash: r.short_sha,
            subject: r.subject,
            author: `${r.author_name} <${r.author_email}>`,
            date: r.authored_at.slice(0, 10),
            identity_mismatch: r.identity_mismatch,
            beyond_upstream: r.reach === "beyond_upstream",
          })),
          error: "",
        };
      } catch (e) {
        if (history?.id !== id) return;
        const msg = errMessage(e);
        history = /does not have any commits|bad default revision/i.test(msg)
          ? { id, status: "ok", entries: [], error: "" }
          : { id, status: "error", entries: [], error: msg };
      }
      if (tab === "history" && repo?.id === id && isOpen()) render();
    };
    if (b.exists && (!history || history.id !== b.id || history.status === "error")) void loadHistory();
    else render();
    const off = onSessionChange((id, what) => {
      if (what === "all" && id === repo?.id && history?.status === "ok") render();
    });
    return { refresh: render, dispose: off };
  }

  function openCommitMenu(btn: HTMLButtonElement, hash: string): void {
    const b = current();
    const def = defaultBranchOf(b.id);
    const target = def && def.name !== b.branch ? def.name : null;
    openMenu(btn, {
      label: `Ações do commit ${hash.slice(0, 7)}`,
      items: [
        {
          id: "merge",
          html: `${icon("merge", 14)}<span>${target ? `Levar para ${esc(target)}…` : "Levar este commit para outro branch…"}</span>`,
          disabled: !b.branch || !b.exists,
        },
        { id: "copy", html: `${icon("copy", 14)}<span>Copiar o código do commit</span>` },
      ],
      onPick: (id) => {
        if (id === "merge" && b.branch) {
          openMerge(b.branch, target, hash);
        } else if (id === "copy") {
          navigator.clipboard.writeText(hash).then(
            () => showToast("Código do commit copiado.", "ok"),
            (e) => showToast(`Não foi possível copiar: ${errMessage(e)}`, "err")
          );
        }
      },
    });
  }

  // ---------- Configuração ----------

  function identityCallout(b: RepoRow): string {
    const id = b.identity;
    if (!b.exists) return "";
    const expected = esc(identityText(id.expected_name, id.expected_email));
    switch (id.state) {
      case "correct":
        return `<div class="callout success">${icon("check")}<span><strong>Identidade correta.</strong> Os commits usam <span class="mono">${esc(
          identityText(id.local_name, id.local_email)
        )}</span>.${id.expected_email ? "" : " A conta não tem e-mail de commit definido. Escolha um em Contas."}</span></div>`;
      case "different":
        return `<div class="callout warning">${icon("alert")}<div>
          <p><strong>Identidade diferente da conta.</strong> Os próximos commits não vão ficar no nome de ${esc(who(b.account_id))}.</p>
          <dl class="drawer-identity-diff">
            <dt>Nesta pasta</dt><dd class="mono">${esc(identityText(id.local_name, id.local_email))}</dd>
            <dt>Da conta</dt><dd class="mono">${expected}</dd>
          </dl>
          <button type="button" class="control" data-action="fix-identity">${icon("check")}<span>Corrigir identidade</span></button>
        </div></div>`;
      default:
        return `<div class="callout warning">${icon("alert")}<div>
          <p><strong>Identidade não definida.</strong> Os commits usam a identidade global do Git em vez de <span class="mono">${expected}</span>.</p>
          <button type="button" class="control" data-action="fix-identity">${icon("check")}<span>Aplicar identidade da conta</span></button>
        </div></div>`;
    }
  }

  function mountConfig(): { refresh: () => void; dispose: () => void } {
    const render = (): void => {
      const b = current();
      const focusKey = panel.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.action : undefined;
      const acc = accountOf(b);
      panel.innerHTML = `<div class="drawer-stack">
        <section class="drawer-section" aria-labelledby="cfg-account">
          <h3 id="cfg-account">${icon("user")}Conta vinculada</h3>
          <div class="settings-row">
            <div class="drawer-config-account">${
              b.account_missing
                ? `<span class="state error">${icon("x")}Conta removida</span>`
                : `${profileAvatar(acc ?? profileFor(accounts, b), 31)}<div><strong>${esc(profileName(acc ?? profileFor(accounts, b)))}</strong>${
                    acc?.login ? `<p class="caption muted">@${esc(acc.login)}</p>` : ""
                  }</div>${acc && !acc.has_token ? `<span class="state warning">${icon("alert")}Desconectada</span>` : ""}`
            }</div>
            <span class="repos-menu-host"><button type="button" class="control" data-action="change-account" aria-haspopup="menu" aria-expanded="false"${
              accounts.length === 0 ? " disabled" : ""
            }>${icon("user")}<span>Trocar conta</span></button></span>
          </div>
          <p class="caption muted drawer-config-help">Commits e envios desta pasta usam esta conta automaticamente.</p>
        </section>

        <section class="drawer-section" aria-labelledby="cfg-identity">
          <h3 id="cfg-identity">${icon("shield")}Identidade do Git</h3>
          ${identityCallout(b)}
          ${identityDiagBlock(b)}
          <dl class="drawer-config-grid">
            <div><dt class="meta-label">user.name</dt><dd class="mono">${b.identity.local_name ? esc(b.identity.local_name) : `<span class="muted">não definido</span>`}</dd></div>
            <div><dt class="meta-label">user.email</dt><dd class="mono">${b.identity.local_email ? esc(b.identity.local_email) : `<span class="muted">não definido</span>`}</dd></div>
          </dl>
        </section>

        <section class="drawer-section" aria-labelledby="cfg-history">
          <h3 id="cfg-history">${icon("history")}Alterações do GitPass</h3>
          <div data-ref="cfg-history-list"><p class="caption muted">Carregando…</p></div>
        </section>

        <section class="drawer-section" aria-labelledby="cfg-folder">
          <h3 id="cfg-folder">${icon("folder")}Pasta e GitHub</h3>
          <div class="repo-meta">
            ${metaField(
              "Pasta local",
              b.root_path,
              "folder",
              `<button type="button" class="icon-button ghost" data-action="copy-path" aria-label="Copiar a pasta local" title="Copiar a pasta local">${icon(
                "copy"
              )}</button><button type="button" class="icon-button ghost" data-action="finder"${b.exists ? "" : " disabled"} aria-label="Mostrar no Finder" title="Mostrar no Finder">${icon(
                "external"
              )}</button>`
            )}
            ${
              b.remote_url
                ? metaField(
                    "Endereço remoto",
                    b.remote_url.replace(/^https:\/\//, ""),
                    "link",
                    `<a class="icon-button ghost" href="${esc(b.remote_url)}" aria-label="Abrir no GitHub (abre no navegador)" title="Abrir no GitHub">${icon("external")}</a>`
                  )
                : `<div class="meta-field">${icon("link")}<div><div class="meta-label">Endereço remoto</div><span class="state warning">${icon(
                    "alert"
                  )}Origin desconhecido</span></div></div>`
            }
            ${metaField("Branch atual", branchText(b), "branch")}
          </div>
          ${
            b.remote_has_credentials
              ? `<p class="caption err drawer-config-help">${icon("lock", 12)} O endereço tem uma senha ou chave escrita nele. Troque por <span class="mono">${esc(
                  b.remote_url ?? ""
                )}</span>.</p>`
              : ""
          }
        </section>

        <section class="drawer-section drawer-danger" aria-labelledby="cfg-unbind">
          <h3 id="cfg-unbind">${icon("x")}Desvincular</h3>
          <div class="settings-row">
            <p class="caption muted">O GitPass deixa de escolher a conta neste repositório. A pasta, os arquivos e o histórico não mudam.</p>
            <button type="button" class="control danger" data-action="unbind">${icon("x")}<span>Desvincular repositório…</span></button>
          </div>
        </section>
      </div>`;
      if (focusKey) panel.querySelector<HTMLElement>(`[data-action="${focusKey}"]`)?.focus();
    };
    const loadConfigHistory = async (bindingId: string): Promise<void> => {
      const el = panel.querySelector<HTMLElement>('[data-ref="cfg-history-list"]');
      if (!el) return;
      try {
        const rows = (await reposApi.configHistory()).filter((r) => r.binding_id === bindingId).slice(0, 6);
        if (!rows.length) {
          el.innerHTML = `<p class="caption muted">Nenhuma troca de conta ou correção de identidade registrada ainda.</p>`;
          return;
        }
        el.innerHTML = `<ul class="config-history-list">${rows
          .map((r) => {
            const op = r.op === "change_account" ? "Trocar conta" : "Corrigir identidade";
            return `<li><div><strong>${esc(op)}</strong><span class="caption muted">${esc(relativeTime(r.at))} · ${esc(r.status)}</span></div>${
              r.undoable
                ? `<button type="button" class="control sm" data-action="undo-history" data-record="${esc(r.id)}">${icon("refresh")}<span>Desfazer</span></button>`
                : ""
            }</li>`;
          })
          .join("")}</ul>`;
      } catch (e) {
        el.innerHTML = `<p class="caption err">${esc(errMessage(e))}</p>`;
      }
    };

    const b = current();
    if (b.exists && (!identityDiag || identityDiag.id !== b.id || identityDiag.status === "error")) {
      void loadIdentityDiagnostic(b.id);
    }
    render();
    void loadConfigHistory(b.id);
    return { refresh: () => {
      render();
      void loadConfigHistory(current().id);
    }, dispose: () => {} };
  }

  // ---------- Ações ----------

  function pickAccount(btn: HTMLButtonElement): void {
    const b = current();
    if (accounts.length === 0) return;
    openAccountMenu(btn, accounts, "Usar neste repositório", b.account_id, (id) => void changeAccount(btn, b, id));
  }

  async function changeAccount(btn: HTMLButtonElement, b: RepoRow, accountId: string): Promise<boolean> {
    try {
      const row = await withBusy(btn, "Trocando…", () => reposApi.changeAccount(b.id, accountId));
      showToast(`${repoName(row)} agora usa ${who(accountId)}. Identidade do Git atualizada.`, "ok");
      onRepoUpdated(row);
      void refreshOutgoing(row.id, true);
      if (showsIdentityDiagnostic()) void loadIdentityDiagnostic(row.id, { remote: identityDiag?.remote ?? null });
      return true;
    } catch (e) {
      showToast(errMessage(e), "err");
      return false;
    }
  }

  function markIdentityUpdated(): void {
    const box = panel.querySelector<HTMLElement>(".identity");
    if (box) box.setAttribute("data-updated", "");
  }

  async function fixIdentity(btn: HTMLButtonElement): Promise<void> {
    const b = current();
    try {
      const row = await withBusy(btn, "Corrigindo…", () => reposApi.fixIdentity(b.id));
      showToast(
        row.identity.state === "correct"
          ? `Identidade aplicada: ${identityText(row.identity.local_name, row.identity.local_email)}.`
          : "Identidade aplicada, mas a conta não tem e-mail de commit. Escolha um em Contas.",
        row.identity.state === "correct" ? "ok" : "err"
      );
      onRepoUpdated(row);
      markIdentityUpdated();
      void loadIdentityDiagnostic(row.id, { remote: identityDiag?.remote ?? null });
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  }

  async function copyText(btn: HTMLButtonElement | null, text: string, done: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      showToast(done, "ok");
      if (btn?.classList.contains("icon-button")) {
        const prev = btn.innerHTML;
        btn.innerHTML = icon("check");
        window.setTimeout(() => {
          if (btn.isConnected) btn.innerHTML = prev;
        }, 1600);
      }
    } catch (e) {
      showToast(`Não foi possível copiar: ${errMessage(e)}`, "err");
    }
  }

  async function setPin(btn: HTMLButtonElement, id: string | null): Promise<void> {
    btn.disabled = true;
    try {
      await pinRepo(id);
      showToast(id ? "Repositório fixado como atual." : "Repositório desafixado.", "ok");
    } catch (e) {
      showToast(errMessage(e), "err");
    } finally {
      if (btn.isConnected) btn.disabled = false;
    }
  }

  async function unbind(): Promise<void> {
    const b = current();
    const reset = await openModal<boolean>({
      title: `Desvincular ${repoName(b)}?`,
      body: `
        <p class="repos-modal-lead">O GitPass deixa de escolher a conta neste repositório. A pasta, os arquivos e o histórico não mudam.</p>
        ${
          b.exists && (b.identity.local_name || b.identity.local_email)
            ? `<label class="repos-check">
                <input type="checkbox" name="reset" />
                <span>Remover também o <span class="mono">user.name</span> e o <span class="mono">user.email</span> que o GitPass aplicou</span>
              </label>`
            : ""
        }`,
      confirmLabel: "Desvincular",
      danger: true,
      read: (d) => d.querySelector<HTMLInputElement>('input[name="reset"]')?.checked ?? false,
    });
    if (reset === null || repo?.id !== b.id) return;
    try {
      await reposApi.unbind(b.id, reset);
      forgetSession(b.id);
      forgetBranches(b.id);
      repos = repos.filter((r) => r.id !== b.id);
      showToast(`${repoName(b)} foi desvinculado.`, "ok");
      ctx.notifyDataChanged();
      hooks.requestClose();
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  }

  el.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    const tabBtn = target.closest<HTMLButtonElement>('[role="tab"][data-tab]');
    if (tabBtn && tabsEl.contains(tabBtn)) {
      selectTab(tabBtn.dataset.tab as Tab, false);
      return;
    }
    const btn = target.closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled) return;
    const action = btn.dataset.action;
    if (action === "close") {
      hooks.requestClose();
      return;
    }
    if (action === "theme") {
      setTheme(resolvedTheme() === "dark" ? "light" : "dark");
      return;
    }
    if (action === "retry-load") {
      void load();
      return;
    }
    if (!repo) return;
    // Ações das abas Enviar e Branches são tratadas por elas.
    if (btn.closest(".push-root, .branches-root")) return;
    const b = repo;
    switch (action) {
      case "pin":
        void setPin(btn, b.id);
        break;
      case "unpin":
        void setPin(btn, null);
        break;
      case "finder":
        reposApi.openInFinder(b.root_path).catch((err) => showToast(errMessage(err), "err"));
        break;
      case "terminal":
        reposApi.openInTerminal(b.root_path).catch((err) => showToast(errMessage(err), "err"));
        break;
      case "edit-account":
        editing = !editing;
        tabHandle?.refresh();
        if (editing) panel.querySelector<HTMLSelectElement>("#drawer-profile")?.focus();
        break;
      case "cancel-account":
        editing = false;
        tabHandle?.refresh();
        panel.querySelector<HTMLElement>('[data-action="edit-account"]')?.focus();
        break;
      case "change-account":
        pickAccount(btn);
        break;
      case "go-accounts":
        ctx.navigate("accounts");
        break;
      case "fix-identity":
        void fixIdentity(btn);
        break;
      case "undo-history": {
        const recordId = btn.dataset.record;
        if (!recordId) return;
        void withBusy(btn, "Desfazendo…", async () => {
          await reposApi.undoConfigHistory(recordId);
          showToast("Alteração desfeita.", "ok");
          ctx.notifyDataChanged();
          tabHandle?.refresh();
        }).catch((err) => showToast(errMessage(err), "err"));
        break;
      }
      case "diag-retry":
        void loadIdentityDiagnostic(b.id, { showOrigins: identityDiag?.showOrigins, remote: identityDiag?.remote ?? null });
        break;
      case "diag-origins":
        if (identityDiag?.id === b.id) {
          identityDiag = { ...identityDiag, showOrigins: !identityDiag.showOrigins };
          tabHandle?.refresh();
        }
        break;
      case "diag-verify-token":
        void loadIdentityDiagnostic(b.id, {
          verifyToken: true,
          showOrigins: identityDiag?.showOrigins,
          remote: identityDiag?.remote ?? null,
        });
        break;
      case "copy-path":
        void copyText(btn, b.root_path, "Pasta local copiada.");
        break;
      case "copy-remote":
        if (b.remote_url) void copyText(btn, b.remote_url, "Endereço remoto copiado.");
        break;
      case "unbind":
        void unbind();
        break;
      case "commit-menu":
        if (btn.dataset.hash) openCommitMenu(btn, btn.dataset.hash);
        break;
    }
  });

  el.addEventListener("change", (e) => {
    const sel = (e.target as HTMLElement).closest<HTMLSelectElement>("select[data-action='diag-remote']");
    if (!sel || !repo) return;
    void loadIdentityDiagnostic(repo.id, { remote: sel.value, showOrigins: identityDiag?.showOrigins });
  });

  el.addEventListener("submit", (e) => {
    const form = (e.target as HTMLElement).closest<HTMLFormElement>('[data-ref="profile-editor"]');
    if (!form || !repo) return;
    e.preventDefault();
    const b = repo;
    const next = form.querySelector<HTMLSelectElement>("#drawer-profile")?.value;
    const submit = form.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    if (!next) return;
    if (next === b.account_id) {
      editing = false;
      tabHandle?.refresh();
      panel.querySelector<HTMLElement>('[data-action="edit-account"]')?.focus();
      return;
    }
    void changeAccount(submit, b, next).then((ok) => {
      if (!ok || repo?.id !== b.id) return;
      editing = false;
      if (tab === "overview") {
        tabHandle?.refresh();
        markIdentityUpdated();
        panel.querySelector<HTMLElement>('[data-action="edit-account"]')?.focus({ preventScroll: true });
      }
    });
  });

  el.addEventListener("keydown", (e) => {
    const tabBtn = (e.target as HTMLElement).closest<HTMLElement>('[role="tab"][data-tab]');
    if (!tabBtn) return;
    const i = TABS.findIndex((t) => t.id === tabBtn.dataset.tab);
    let next = -1;
    if (e.key === "ArrowRight") next = (i + 1) % TABS.length;
    else if (e.key === "ArrowLeft") next = (i - 1 + TABS.length) % TABS.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = TABS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    selectTab(TABS[next].id, true);
  });

  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !isOpen() || e.defaultPrevented) return;
    if (document.querySelector(".modal-backdrop, .gp-popover, .repos-menu")) return;
    e.preventDefault();
    if (editing) {
      editing = false;
      tabHandle?.refresh();
      panel.querySelector<HTMLElement>('[data-action="edit-account"]')?.focus();
      return;
    }
    hooks.requestClose();
  });

  heroEl.addEventListener("pointermove", (e) => {
    if (e.pointerType !== "mouse" || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const r = heroEl.getBoundingClientRect();
    heroEl.style.setProperty("--glint-x", `${((e.clientX - r.left) / r.width) * 100}%`);
    heroEl.style.setProperty("--glint-y", `${((e.clientY - r.top) / r.height) * 100}%`);
  });

  document.addEventListener(THEME_CHANGED_EVENT, () => {
    if (isOpen()) renderToolbar();
  });
  document.addEventListener(DATA_CHANGED_EVENT, () => scheduleReload());
  onSessionChange((id, what) => {
    if (!isOpen() || id !== repo?.id) return;
    if (what === "pushed") scheduleReload();
    else if (what === "all" && repo) {
      renderTabs();
      renderHero();
    }
  });
  reposApi
    .onOutgoingChanged((id) => {
      if (!isOpen()) return;
      if (id === repo?.id) {
        scheduleReload();
        if (!isPushing(id)) void refreshOutgoing(id, false);
      }
    })
    .catch(() => {});
  onSystemState((s) => {
    const next = s.current.status === "ok" ? s.current.value : null;
    if (next === system) return;
    system = next;
    if (isOpen() && repo && tab === "overview") tabHandle?.refresh();
  });

  return { open, close, isOpen };
}
