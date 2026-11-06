import "./cloud.css";
import { api, type AccountRow, type BindingRow, type GithubRepoSummary } from "../lib/api";
import { esc, relativeTime, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileChip, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView } from "../lib/view";

const PAGE_SIZE = 50;

type LocalFilter = "all" | "local" | "cloud";

type RepoState =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ok"; repos: GithubRepoSummary[] };

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
  let profileFilter = "all";
  let generation = 0;
  let alive = true;

  root.innerHTML = `
    <header class="view-header">
      <div>
        <h1 class="view-title">Na cloud</h1>
        <p class="view-sub">Repositórios no GitHub, por perfil.</p>
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
      <div class="segmented" id="cloud-profile-filter" role="group" aria-label="Filtrar por perfil" hidden></div>
    </div>
    <div class="cloud-body" id="cloud-body"></div>
  `;

  const refreshBtn = root.querySelector<HTMLButtonElement>("#cloud-refresh")!;
  const toolbar = root.querySelector<HTMLElement>("#cloud-toolbar")!;
  const searchInput = root.querySelector<HTMLInputElement>("#cloud-search")!;
  const localGroup = root.querySelector<HTMLElement>("#cloud-local-filter")!;
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

  async function fetchRepos(account: AccountRow, gen: number): Promise<void> {
    repoStates.set(account.id, { status: "loading" });
    renderBody();
    try {
      const repos = await api.cloudRepos(account.id);
      if (!alive || gen !== generation) return;
      repoStates.set(account.id, { status: "ok", repos });
    } catch (e) {
      if (!alive || gen !== generation) return;
      repoStates.set(account.id, { status: "error", message: String(e) });
    }
    renderBody();
  }

  async function loadAll(): Promise<void> {
    const gen = ++generation;
    try {
      const [accs, binds] = await Promise.all([api.accounts(), api.bindings()]);
      if (!alive || gen !== generation) return;
      accounts = accs;
      bindings = binds;
      loadError = null;
    } catch (e) {
      if (!alive || gen !== generation) return;
      loadError = String(e);
      loadedAccounts = true;
      renderAll();
      return;
    }
    loadedAccounts = true;
    const ids = new Set(accounts.map((a) => a.id));
    for (const id of [...repoStates.keys()]) if (!ids.has(id)) repoStates.delete(id);
    if (profileFilter !== "all" && !ids.has(profileFilter)) profileFilter = "all";
    for (const a of accounts) {
      if (a.has_token) repoStates.set(a.id, { status: "loading" });
      else repoStates.delete(a.id);
    }
    renderAll();
    await Promise.all(accounts.filter((a) => a.has_token).map((a) => fetchRepos(a, gen)));
  }

  // ---------- Renderização ----------

  function renderToolbar(): void {
    toolbar.hidden = accounts.length === 0;
    for (const btn of localGroup.querySelectorAll<HTMLButtonElement>("button")) {
      btn.setAttribute("aria-pressed", String(btn.dataset.local === localFilter));
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

  function repoRow(r: GithubRepoSummary, isLocal: boolean): string {
    const privateBadge = r.private
      ? `<span class="badge cloud-badge-private">${icon("lock", 12)}Privado</span>`
      : "";
    const localBadge = isLocal ? `<span class="badge success">${icon("check", 12)}Clone local</span>` : "";
    const branch = r.default_branch
      ? `<span class="cloud-branch mono muted small" title="Ramo por omissão">${icon("branch", 13)}<span class="truncate">${esc(r.default_branch)}</span></span>`
      : "";
    const pushed = r.pushed_at
      ? `<span class="cloud-pushed muted small" title="Último push">${esc(relativeTime(r.pushed_at))}</span>`
      : "";
    return `<li><a class="list-row cloud-row" href="${esc(r.html_url)}" target="_blank" rel="noopener noreferrer">
      <span class="cloud-row-name mono truncate">${esc(r.full_name)}</span>
      <span class="cloud-row-meta">${privateBadge}${localBadge}${branch}</span>
      ${pushed}
      <span class="cloud-external muted" aria-label="Abrir no GitHub">${icon("external", 14)}</span>
    </a></li>`;
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
      const isLocal = locals.has(r.full_name.toLowerCase());
      if (localFilter === "local" && !isLocal) return false;
      if (localFilter === "cloud" && isLocal) return false;
      return true;
    });
    const filtering = q !== "" || localFilter !== "all";
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
      content = `<ul class="list">${visible.map((r) => repoRow(r, locals.has(r.full_name.toLowerCase()))).join("")}</ul>
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
  }

  // ---------- Eventos ----------

  const onSearch = () => {
    query = searchInput.value;
    renderBody();
  };
  searchInput.addEventListener("input", onSearch);

  localGroup.addEventListener("click", (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-local]");
    if (!btn) return;
    localFilter = btn.dataset.local as LocalFilter;
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
    }
  });

  refreshBtn.addEventListener("click", () => {
    void withBusy(refreshBtn, "A atualizar…", loadAll).catch((e) => showToast(String(e)));
  });

  const onDataChanged = () => void loadAll();
  document.addEventListener(DATA_CHANGED_EVENT, onDataChanged);

  renderBody();
  void loadAll();

  return () => {
    alive = false;
    generation++;
    document.removeEventListener(DATA_CHANGED_EVENT, onDataChanged);
  };
};
