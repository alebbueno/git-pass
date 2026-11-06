import "./cloud.css";
import { listen } from "@tauri-apps/api/event";
import { api, type AccountRow, type BindingRow } from "../lib/api";
import {
  apiCloud,
  CLONE_PROGRESS_EVENT,
  type CloneProgressPayload,
  type CloudRepo,
} from "../lib/api-cloud";
import { errMessage, esc, relativeTime, shortenPath, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileChip, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView } from "../lib/view";

const PAGE_SIZE = 50;

type LocalFilter = "all" | "local" | "cloud";
type PermFilter = "all" | "write";

type RepoState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ok"; repos: CloudRepo[] };

type CloneJob = { accountId: string; fullName: string; phase: string; percent: number };

type Popover = { key: string; accountId: string; fullName: string; parent: string | null };

// ---------- Estado partilhado entre montagens ----------
// A clonagem continua se o utilizador sair desta vista; ao voltar, o progresso reaparece.

const activeClones = new Map<string, CloneJob>();
/** Clones concluídos nesta sessão (a linha passa a "Clone local" antes de a lista recarregar). */
const clonedKeys = new Set<string>();
let sink: { render: () => void; progress: () => void } | null = null;
let listenerStarted = false;

const rowKey = (accountId: string, fullName: string) => `${accountId}|${fullName.toLowerCase()}`;

function ensureProgressListener(): void {
  if (listenerStarted) return;
  listenerStarted = true;
  void listen<CloneProgressPayload>(CLONE_PROGRESS_EVENT, (ev) => {
    const name = ev.payload.full_name.toLowerCase();
    for (const job of activeClones.values()) {
      if (job.fullName.toLowerCase() === name) {
        job.phase = ev.payload.phase;
        job.percent = ev.payload.percent;
      }
    }
    sink?.progress();
  }).catch(() => {
    listenerStarted = false;
  });
}

const PHASE_LABEL: Record<string, string> = {
  starting: "A iniciar…",
  counting: "A contar objetos",
  compressing: "A comprimir objetos",
  receiving: "A receber objetos",
  resolving: "A resolver deltas",
  checkout: "A extrair ficheiros",
  binding: "A vincular à conta",
};

/** Intervalo [início, fim] de cada fase no progresso global (as fases do Git são sequenciais). */
const PHASE_SPAN: Record<string, [number, number]> = {
  starting: [0, 0],
  counting: [0, 5],
  compressing: [5, 10],
  receiving: [10, 80],
  resolving: [80, 95],
  checkout: [95, 100],
  binding: [100, 100],
};

function overallPercent(job: CloneJob): number {
  const [from, to] = PHASE_SPAN[job.phase] ?? [0, 100];
  const p = Math.max(0, Math.min(100, job.percent));
  return Math.round(from + ((to - from) * p) / 100);
}

// ---------- Permissões ----------

const svg = (inner: string) =>
  `<svg class="icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;
const ICON_SHIELD = svg('<path d="M12 3 4 6v6c0 4.5 3.2 8 8 9 4.8-1 8-4.5 8-9V6z"/><path d="m9 12 2 2 4-4"/>');
const ICON_PENCIL = svg('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>');
const ICON_EYE = svg('<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>');
const ICON_ARCHIVE = svg('<rect x="3" y="4" width="18" height="4" rx="1"/><path d="M5 8v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>');

function canWrite(r: CloudRepo): boolean {
  const p = r.permissions;
  return !!p && (p.admin || p.maintain || p.push);
}

function permissionBadge(r: CloudRepo): string {
  const p = r.permissions;
  if (!p) return "";
  if (p.admin) return `<span class="badge success" title="Administrador do repositório">${ICON_SHIELD}Admin</span>`;
  if (p.maintain || p.push) {
    const title = p.maintain && !p.push ? "Manutenção (pode escrever)" : "Pode enviar alterações (push)";
    return `<span class="badge success" title="${title}">${ICON_PENCIL}Escrita</span>`;
  }
  if (p.triage || p.pull) return `<span class="badge" title="Só leitura">${ICON_EYE}Leitura</span>`;
  return "";
}

export const mount: MountView = (root, ctx) => {
  // Estado em memória enquanto a tela está montada
  let accounts: AccountRow[] = [];
  let bindings: BindingRow[] = [];
  let loadedAccounts = false;
  let loadError: string | null = null;
  const repoStates = new Map<string, RepoState>();
  const expanded = new Set<string>();
  let query = "";
  let localFilter: LocalFilter = "all";
  let permFilter: PermFilter = "all";
  let profileFilter = "all";
  let popover: Popover | null = null;
  let focusPopover = false;
  let generation = 0;
  let alive = true;

  root.innerHTML = `
    <header class="view-header">
      <div>
        <h1 class="view-title">Na cloud</h1>
        <p class="view-sub">Repositórios no GitHub, por perfil. Clona com a conta certa.</p>
      </div>
      <div class="view-actions">
        <button type="button" class="btn ghost" id="cloud-refresh">${icon("refresh")}<span>Atualizar</span></button>
      </div>
    </header>
    <div class="cloud-toolbar" id="cloud-toolbar" hidden>
      <label class="cloud-search">
        <span class="cloud-search-icon">${icon("search")}</span>
        <input type="search" class="input" id="cloud-search" placeholder="Filtrar por nome (dono/repositório)" aria-label="Filtrar repositórios por nome" autocomplete="off" spellcheck="false" />
      </label>
      <div class="segmented" id="cloud-local-filter" role="group" aria-label="Filtrar por clone local">
        <button type="button" data-local="all" aria-pressed="true">Todos</button>
        <button type="button" data-local="local" aria-pressed="false">Com clone local</button>
        <button type="button" data-local="cloud" aria-pressed="false">Só na cloud</button>
      </div>
      <div class="segmented" id="cloud-perm-filter" role="group" aria-label="Filtrar por permissão">
        <button type="button" data-perm="all" aria-pressed="true">Qualquer acesso</button>
        <button type="button" data-perm="write" aria-pressed="false">Posso escrever</button>
      </div>
      <div class="segmented" id="cloud-profile-filter" role="group" aria-label="Filtrar por perfil" hidden></div>
    </div>
    <div class="cloud-body" id="cloud-body"></div>
  `;

  const refreshBtn = root.querySelector<HTMLButtonElement>("#cloud-refresh")!;
  const toolbar = root.querySelector<HTMLElement>("#cloud-toolbar")!;
  const searchInput = root.querySelector<HTMLInputElement>("#cloud-search")!;
  const localGroup = root.querySelector<HTMLElement>("#cloud-local-filter")!;
  const permGroup = root.querySelector<HTMLElement>("#cloud-perm-filter")!;
  const profileGroup = root.querySelector<HTMLElement>("#cloud-profile-filter")!;
  const body = root.querySelector<HTMLElement>("#cloud-body")!;

  // ---------- Dados ----------

  function localKeys(accountId: string): Set<string> {
    const keys = new Set<string>();
    for (const b of bindings) {
      if (b.account_id === accountId && b.github_owner && b.github_repo) {
        keys.add(`${b.github_owner}/${b.github_repo}`.toLowerCase());
      }
    }
    return keys;
  }

  function isLocalRepo(accountId: string, r: CloudRepo, locals: Set<string>): boolean {
    return !!r.local_binding_id || locals.has(r.full_name.toLowerCase()) || clonedKeys.has(rowKey(accountId, r.full_name));
  }

  async function fetchRepos(account: AccountRow, gen: number, silent = false): Promise<void> {
    const prev = repoStates.get(account.id);
    if (!silent || !prev || prev.status !== "ok") {
      repoStates.set(account.id, { status: "loading" });
      renderBody();
    }
    try {
      const repos = await apiCloud.repos(account.id);
      if (!alive || gen !== generation) return;
      repoStates.set(account.id, { status: "ok", repos });
    } catch (e) {
      if (!alive || gen !== generation) return;
      if (silent && prev?.status === "ok") {
        // Mantém a lista anterior em vez de a trocar por um erro numa atualização em segundo plano.
        repoStates.set(account.id, prev);
        showToast(`Não foi possível atualizar os repositórios de ${profileName(account)}: ${errMessage(e)}`);
      } else {
        repoStates.set(account.id, { status: "error", message: errMessage(e) });
      }
    }
    renderBody();
  }

  /**
   * Carrega contas e vínculos; as listas já carregadas ficam visíveis enquanto atualizam.
   * `refetch = false` (mudanças de configuração) só vai ao GitHub por contas sem lista válida: os vínculos
   * novos chegam por `api.bindings()` e `clonedKeys`, sem repetir centenas de pedidos à API.
   */
  async function loadAll(refetch = true): Promise<void> {
    const gen = ++generation;
    try {
      const [accs, binds] = await Promise.all([api.accounts(), api.bindings()]);
      if (!alive || gen !== generation) return;
      accounts = accs;
      bindings = binds;
      loadError = null;
    } catch (e) {
      if (!alive || gen !== generation) return;
      loadError = errMessage(e);
      loadedAccounts = true;
      renderAll();
      return;
    }
    loadedAccounts = true;
    const ids = new Set(accounts.map((a) => a.id));
    for (const id of [...repoStates.keys()]) if (!ids.has(id)) repoStates.delete(id);
    if (profileFilter !== "all" && !ids.has(profileFilter)) profileFilter = "all";
    for (const a of accounts) {
      if (!a.has_token) repoStates.delete(a.id);
      else if (!repoStates.has(a.id)) repoStates.set(a.id, { status: "loading" });
    }
    renderAll();
    const toFetch = accounts.filter((a) => a.has_token && (refetch || repoStates.get(a.id)?.status !== "ok"));
    await Promise.all(toFetch.map((a) => fetchRepos(a, gen, true)));
  }

  // ---------- Clonagem ----------

  function destLabel(p: Popover): string {
    const [owner, name] = p.fullName.split("/");
    return p.parent ? `${shortenPath(p.parent).replace(/\/$/, "")}/${name}` : `~/GitHub/${owner}/${name}`;
  }

  async function startClone(p: Popover): Promise<void> {
    const key = p.key;
    if (activeClones.has(key)) return;
    activeClones.set(key, { accountId: p.accountId, fullName: p.fullName, phase: "starting", percent: 0 });
    popover = null;
    renderBody();
    try {
      const path = await apiCloud.clone(p.accountId, p.fullName, p.parent);
      clonedKeys.add(key);
      showToast(`${p.fullName} clonado em ${shortenPath(path)}.`, "ok");
    } catch (e) {
      showToast(errMessage(e));
    } finally {
      activeClones.delete(key);
      sink?.render();
    }
  }

  // ---------- Renderização ----------

  function renderToolbar(): void {
    toolbar.hidden = accounts.length === 0;
    for (const btn of localGroup.querySelectorAll<HTMLButtonElement>("button")) {
      btn.setAttribute("aria-pressed", String(btn.dataset.local === localFilter));
    }
    for (const btn of permGroup.querySelectorAll<HTMLButtonElement>("button")) {
      btn.setAttribute("aria-pressed", String(btn.dataset.perm === permFilter));
    }
    if (accounts.length > 1) {
      profileGroup.hidden = false;
      const opts = [{ id: "all", label: "Todos os perfis" }, ...accounts.map((a) => ({ id: a.id, label: profileName(a) }))];
      profileGroup.innerHTML = opts
        .map((o) => `<button type="button" data-profile="${esc(o.id)}" aria-pressed="${o.id === profileFilter}">${esc(o.label)}</button>`)
        .join("");
    } else {
      profileGroup.hidden = true;
      profileGroup.innerHTML = "";
    }
  }

  function renderAll(): void {
    renderToolbar();
    renderBody();
  }

  function progressHtml(job: CloneJob): string {
    const pct = overallPercent(job);
    const label = PHASE_LABEL[job.phase] ?? "A clonar…";
    return `<div class="cloud-progress-wrap" data-clone-key="${esc(rowKey(job.accountId, job.fullName))}">
      <div class="cloud-progress" role="progressbar" aria-label="A clonar ${esc(job.fullName)}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct}">
        <span class="cloud-progress-fill" style="width:${pct}%"></span>
      </div>
      <span class="cloud-progress-label small muted"><span class="cloud-progress-phase">${esc(label)}</span> · <span class="cloud-progress-pct">${pct}%</span></span>
    </div>`;
  }

  function popoverHtml(p: Popover, account: AccountRow): string {
    return `<div class="cloud-popover" role="dialog" aria-label="Clonar ${esc(p.fullName)}">
      <div class="field-label">Destino</div>
      <div class="cloud-dest mono small" title="${esc(destLabel(p))}">${icon("folder", 14)}<span class="truncate">${esc(destLabel(p))}</span></div>
      <div class="cloud-popover-choose">
        <button type="button" class="btn" data-pop-choose>${icon("folder")}<span>Escolher pasta…</span></button>
        ${p.parent ? `<button type="button" class="btn ghost" data-pop-reset>Usar a pasta por omissão</button>` : ""}
      </div>
      <p class="muted small cloud-popover-hint">Clona com o perfil ${esc(profileName(account))} e vincula a pasta a este perfil.</p>
      <div class="cloud-popover-actions">
        <button type="button" class="btn ghost" data-pop-cancel>Cancelar</button>
        <button type="button" class="btn primary" data-pop-confirm>${icon("cloud")}<span>Clonar</span></button>
      </div>
    </div>`;
  }

  function repoRow(a: AccountRow, r: CloudRepo, isLocal: boolean): string {
    const key = rowKey(a.id, r.full_name);
    const job = activeClones.get(key);

    const privateBadge = r.private
      ? `<span class="badge cloud-badge-private">${icon("lock", 12)}Privado</span>`
      : "";
    const archivedBadge = r.archived
      ? `<span class="badge warning" title="Repositório arquivado (só leitura)">${ICON_ARCHIVE}Arquivado</span>`
      : "";
    const forkBadge = r.fork ? `<span class="badge" title="Fork de outro repositório">${icon("branch", 12)}Fork</span>` : "";
    const localBadge = isLocal && !job ? `<span class="badge success">${icon("check", 12)}Clone local</span>` : "";
    const branch = r.default_branch
      ? `<span class="cloud-branch mono muted small" title="Ramo por omissão">${icon("branch", 13)}<span class="truncate">${esc(r.default_branch)}</span></span>`
      : "";
    const pushed = r.pushed_at
      ? `<span class="cloud-pushed muted small" title="Último push">${esc(relativeTime(r.pushed_at))}</span>`
      : "";
    const desc = r.description
      ? `<span class="cloud-desc muted small truncate" title="${esc(r.description)}">${esc(r.description)}</span>`
      : "";

    let action = "";
    if (job) {
      action = `<span class="badge cloud-cloning" role="status"><span class="spinner cloud-mini-spinner" aria-hidden="true"></span>A clonar</span>`;
    } else if (isLocal) {
      action = `<button type="button" class="btn" data-open-local>${icon("repo")}<span>Abrir</span></button>`;
    } else {
      const open = popover?.key === key;
      action = `<button type="button" class="btn primary" data-clone-open aria-haspopup="dialog" aria-expanded="${open}">${icon("plus")}<span>Clonar</span></button>`;
    }

    const pop = popover?.key === key && !job && !isLocal ? popoverHtml(popover, a) : "";

    return `<li class="cloud-item" data-key="${esc(key)}" data-account="${esc(a.id)}" data-full="${esc(r.full_name)}">
      <div class="list-row cloud-row">
        <div class="cloud-row-main">
          <span class="cloud-row-name mono truncate" title="${esc(r.full_name)}">${esc(r.full_name)}</span>
          ${desc}
        </div>
        <span class="cloud-row-meta">${permissionBadge(r)}${privateBadge}${archivedBadge}${forkBadge}${localBadge}${branch}</span>
        ${pushed}
        <span class="cloud-actions">
          ${action}
          <button type="button" class="btn ghost icon-only" data-open-url="${esc(r.html_url)}" aria-label="Abrir ${esc(r.full_name)} no GitHub" title="Abrir no GitHub">${icon("external", 15)}</button>
        </span>
        ${job ? progressHtml(job) : ""}
      </div>
      ${pop}
    </li>`;
  }

  function sectionHead(a: AccountRow, countText: string, extra = ""): string {
    return `<div class="cloud-section-head">
      ${profileChip(a, { size: "md" })}
      ${a.login ? `<span class="mono muted small">@${esc(a.login)}</span>` : ""}
      ${countText ? `<span class="cloud-count muted small">${esc(countText)}</span>` : ""}
      ${extra}
    </div>`;
  }

  function sectionHtml(a: AccountRow): string {
    if (!a.has_token) {
      return `<section class="cloud-section" aria-label="${esc(profileName(a))}">
        ${sectionHead(a, "", `<span class="badge warning">${icon("alert", 12)}Sem token</span>`)}
        <div class="cloud-notoken">
          <p class="muted small">Este perfil não tem token, por isso não é possível listar os repositórios.</p>
          <button type="button" class="btn" data-action="accounts">${icon("key")}<span>Gerir contas</span></button>
        </div>
      </section>`;
    }
    const state = repoStates.get(a.id) ?? { status: "loading" };
    if (state.status === "loading") {
      return `<section class="cloud-section" aria-label="${esc(profileName(a))}" aria-busy="true">
        ${sectionHead(a, "", `<span class="spinner cloud-head-spinner" role="status" aria-label="A carregar"></span>`)}
        <div class="cloud-skeletons">${'<div class="skeleton cloud-skeleton-row"></div>'.repeat(3)}</div>
      </section>`;
    }
    if (state.status === "error") {
      return `<section class="cloud-section" aria-label="${esc(profileName(a))}">
        ${sectionHead(a, "")}
        <div class="callout error" role="alert">
          ${icon("alert")}
          <div class="cloud-error-body">
            <div><strong>Erro ao obter repositórios.</strong></div>
            <div class="cloud-error-msg">${esc(state.message)}</div>
          </div>
          <button type="button" class="btn" data-retry="${esc(a.id)}">${icon("refresh")}<span>Tentar de novo</span></button>
        </div>
      </section>`;
    }

    const locals = localKeys(a.id);
    const q = query.trim().toLowerCase();
    const filtered = state.repos.filter((r) => {
      if (q && !r.full_name.toLowerCase().includes(q)) return false;
      const isLocal = isLocalRepo(a.id, r, locals);
      if (localFilter === "local" && !isLocal) return false;
      if (localFilter === "cloud" && isLocal) return false;
      if (permFilter === "write" && !canWrite(r)) return false;
      return true;
    });
    const filtering = q !== "" || localFilter !== "all" || permFilter !== "all";
    const total = state.repos.length;
    const countText = filtering
      ? `${filtered.length} de ${total} ${total === 1 ? "repositório" : "repositórios"}`
      : `${total} ${total === 1 ? "repositório" : "repositórios"}`;

    let content: string;
    if (total === 0) {
      content = `<p class="muted small cloud-empty">Sem repositórios nesta conta.</p>`;
    } else if (filtered.length === 0) {
      content = `<p class="muted small cloud-empty">Nenhum repositório corresponde ao filtro.</p>`;
    } else {
      const showAll = expanded.has(a.id);
      const visible = showAll ? filtered : filtered.slice(0, PAGE_SIZE);
      const more = filtered.length - visible.length;
      content = `<ul class="list">${visible.map((r) => repoRow(a, r, isLocalRepo(a.id, r, locals))).join("")}</ul>
        ${more > 0 ? `<div class="cloud-more"><button type="button" class="btn" data-more="${esc(a.id)}">Mostrar mais (${more})</button></div>` : ""}`;
    }
    return `<section class="cloud-section" aria-label="${esc(profileName(a))}">
      ${sectionHead(a, countText)}
      ${content}
    </section>`;
  }

  function renderBody(): void {
    if (!loadedAccounts) {
      body.innerHTML = `<div class="cloud-skeletons">${'<div class="skeleton cloud-skeleton-row"></div>'.repeat(3)}</div>`;
      return;
    }
    if (loadError) {
      body.innerHTML = `<div class="callout error" role="alert">${icon("alert")}<div class="cloud-error-body"><div><strong>Erro ao carregar perfis.</strong></div><div class="cloud-error-msg">${esc(loadError)}</div></div><button type="button" class="btn" data-action="reload">${icon("refresh")}<span>Tentar de novo</span></button></div>`;
      return;
    }
    if (accounts.length === 0) {
      body.innerHTML = `<div class="panel"><div class="empty-state">
        <span class="empty-icon">${icon("cloud", 26)}</span>
        <h3>Sem perfis ligados</h3>
        <p>Ligue um perfil GitHub para ver aqui os repositórios que tem na cloud.</p>
        <button type="button" class="btn primary" data-action="accounts">${icon("plus")}<span>Ligar um perfil</span></button>
      </div></div>`;
      return;
    }
    const shown = accounts.filter((a) => profileFilter === "all" || a.id === profileFilter);
    body.innerHTML = shown.map(sectionHtml).join("");
    if (focusPopover) {
      focusPopover = false;
      body.querySelector<HTMLElement>("[data-pop-confirm]")?.focus();
    }
  }

  /** Atualiza só as barras de progresso (evita reconstruir a lista a cada percentagem). */
  function renderProgress(): void {
    for (const el of body.querySelectorAll<HTMLElement>("[data-clone-key]")) {
      const job = activeClones.get(el.dataset.cloneKey ?? "");
      if (!job) continue;
      const pct = overallPercent(job);
      el.querySelector<HTMLElement>(".cloud-progress")?.setAttribute("aria-valuenow", String(pct));
      const fill = el.querySelector<HTMLElement>(".cloud-progress-fill");
      if (fill) fill.style.width = `${pct}%`;
      const phase = el.querySelector<HTMLElement>(".cloud-progress-phase");
      if (phase) phase.textContent = PHASE_LABEL[job.phase] ?? "A clonar…";
      const pctEl = el.querySelector<HTMLElement>(".cloud-progress-pct");
      if (pctEl) pctEl.textContent = `${pct}%`;
    }
  }

  // ---------- Eventos ----------

  searchInput.addEventListener("input", () => {
    query = searchInput.value;
    renderBody();
  });

  localGroup.addEventListener("click", (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-local]");
    if (!btn) return;
    localFilter = btn.dataset.local as LocalFilter;
    renderToolbar();
    renderBody();
  });

  permGroup.addEventListener("click", (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-perm]");
    if (!btn) return;
    permFilter = btn.dataset.perm as PermFilter;
    renderToolbar();
    renderBody();
  });

  profileGroup.addEventListener("click", (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-profile]");
    if (!btn) return;
    profileFilter = btn.dataset.profile ?? "all";
    renderToolbar();
    renderBody();
  });

  let choosing = false;

  body.addEventListener("click", (ev) => {
    const el = ev.target as HTMLElement;
    const action = el.closest<HTMLElement>("[data-action]")?.dataset.action;
    if (action === "accounts") return ctx.navigate("accounts");
    if (action === "reload") return void loadAll();

    const retry = el.closest<HTMLElement>("[data-retry]")?.dataset.retry;
    if (retry) {
      const acc = accounts.find((a) => a.id === retry);
      if (acc) void fetchRepos(acc, generation);
      return;
    }
    const more = el.closest<HTMLElement>("[data-more]")?.dataset.more;
    if (more) {
      expanded.add(more);
      renderBody();
      return;
    }

    const url = el.closest<HTMLElement>("[data-open-url]")?.dataset.openUrl;
    if (url) {
      api.openExternal(url).catch((e) => showToast(errMessage(e)));
      return;
    }
    if (el.closest("[data-open-local]")) return ctx.navigate("repos");

    const item = el.closest<HTMLElement>(".cloud-item");
    const accountId = item?.dataset.account;
    const fullName = item?.dataset.full;
    const key = item?.dataset.key;

    if (el.closest("[data-clone-open]") && accountId && fullName && key) {
      if (popover?.key === key) {
        popover = null;
        renderBody();
      } else {
        popover = { key, accountId, fullName, parent: null };
        focusPopover = true;
        renderBody();
      }
      return;
    }
    if (el.closest("[data-pop-cancel]")) {
      popover = null;
      renderBody();
      return;
    }
    if (el.closest("[data-pop-reset]") && popover) {
      popover.parent = null;
      renderBody();
      return;
    }
    const chooseBtn = el.closest<HTMLButtonElement>("[data-pop-choose]");
    if (chooseBtn && popover && !choosing) {
      choosing = true;
      const p = popover;
      void withBusy(chooseBtn, "A escolher…", () => apiCloud.chooseFolder())
        .then((folder) => {
          if (folder && alive && popover === p) p.parent = folder;
        })
        .catch((e) => showToast(errMessage(e)))
        .finally(() => {
          choosing = false;
          if (alive && popover === p) {
            focusPopover = true;
            renderBody();
          }
        });
      return;
    }
    if (el.closest("[data-pop-confirm]") && popover) {
      void startClone(popover);
    }
  });

  // Fechar o popover: clique fora ou Escape.
  const onDocClick = (ev: MouseEvent) => {
    if (!popover) return;
    const t = ev.target as HTMLElement | null;
    if (!t || !t.isConnected) return;
    if (t.closest(".cloud-popover") || t.closest("[data-clone-open]")) return;
    popover = null;
    renderBody();
  };
  const onKey = (ev: KeyboardEvent) => {
    if (ev.key === "Escape" && popover) {
      popover = null;
      renderBody();
    }
  };
  document.addEventListener("click", onDocClick);
  document.addEventListener("keydown", onKey);

  refreshBtn.addEventListener("click", () => {
    void withBusy(refreshBtn, "A atualizar…", loadAll).catch((e) => showToast(errMessage(e)));
  });

  const onDataChanged = () => void loadAll(false);
  document.addEventListener(DATA_CHANGED_EVENT, onDataChanged);

  sink = {
    render: () => {
      if (alive) renderBody();
    },
    progress: () => {
      if (alive) renderProgress();
    },
  };
  ensureProgressListener();

  renderBody();
  void loadAll();

  return () => {
    alive = false;
    generation++;
    popover = null;
    document.removeEventListener(DATA_CHANGED_EVENT, onDataChanged);
    document.removeEventListener("click", onDocClick);
    document.removeEventListener("keydown", onKey);
    sink = null;
  };
};
