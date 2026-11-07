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
import { errorView } from "../lib/errors";
import { icon } from "../lib/icons";
import { profileChip, profileFamily, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView } from "../lib/view";
import { drawerRoute } from "./repo-detail";
import { plural } from "./repos-ui";

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
  starting: "Iniciando…",
  counting: "Contando objetos",
  compressing: "Comprimindo objetos",
  receiving: "Recebendo objetos",
  resolving: "Resolvendo deltas",
  checkout: "Extraindo arquivos",
  binding: "Vinculando à conta",
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

function canWrite(r: CloudRepo): boolean {
  const p = r.permissions;
  return !!p && (p.admin || p.maintain || p.push);
}

function permissionTag(r: CloudRepo): string {
  const p = r.permissions;
  if (!p) return "";
  if (p.admin) return `<span class="cloud-tag" title="Administrador do repositório">${icon("shieldCheck", 11)}Admin</span>`;
  if (p.maintain || p.push) {
    const title = p.maintain && !p.push ? "Manutenção (pode escrever)" : "Pode enviar alterações (push)";
    return `<span class="cloud-tag" title="${title}">${icon("edit", 11)}Escrita</span>`;
  }
  if (p.triage || p.pull) return `<span class="cloud-tag" title="Só leitura">${icon("file", 11)}Leitura</span>`;
  return "";
}

type Entry = { account: AccountRow; repo: CloudRepo; local: boolean; bindingId: string | null };

export const mount: MountView = (root, ctx, _param, initialQuery) => {
  let accounts: AccountRow[] = [];
  let bindings: BindingRow[] = [];
  let loadedAccounts = false;
  let loadError: string | null = null;
  const repoStates = new Map<string, RepoState>();
  let showAll = false;
  let query = "";
  let localFilter: LocalFilter = "all";
  let permFilter: PermFilter = "all";
  let profileFilter = initialQuery?.get("profile") ?? "all";
  let popover: Popover | null = null;
  let focusPopover = false;
  let generation = 0;
  let alive = true;

  root.innerHTML = `
    <div class="page-head">
      <div class="heading-group"><h1>Na nuvem</h1><p>Repositórios disponíveis nas suas contas GitHub.</p></div>
      <button type="button" class="icon-button" id="cloud-refresh" aria-label="Atualizar a lista" title="Atualizar">${icon("refresh")}</button>
    </div>
    <div class="filterbar" id="cloud-toolbar" role="search" aria-label="Filtros da nuvem" hidden>
      <label class="search">${icon("search")}<span class="sr-only">Buscar na nuvem</span>
        <input type="search" id="cloud-search" placeholder="Buscar repositório" title="Busque por dono ou nome" autocomplete="off" spellcheck="false" />
      </label>
      <label class="select-wrap" id="cloud-profile-wrap">${icon("user")}<span class="sr-only">Filtrar por perfil</span><select id="cloud-profile"></select></label>
      <label class="select-wrap">${icon("folder")}<span class="sr-only">Filtrar por clone local</span>
        <select id="cloud-local">
          <option value="all">Todos</option>
          <option value="local">Com clone local</option>
          <option value="cloud">Só na nuvem</option>
        </select>
      </label>
      <button type="button" class="control filter-toggle" id="cloud-write" aria-pressed="false">${icon("edit")}<span>Posso escrever</span></button>
      <span class="filter-count" id="cloud-count" role="status" aria-live="polite"></span>
    </div>
    <div class="cloud-body" id="cloud-body"></div>
  `;

  const refreshBtn = root.querySelector<HTMLButtonElement>("#cloud-refresh")!;
  const toolbar = root.querySelector<HTMLElement>("#cloud-toolbar")!;
  const searchInput = root.querySelector<HTMLInputElement>("#cloud-search")!;
  const profileSelect = root.querySelector<HTMLSelectElement>("#cloud-profile")!;
  const profileWrap = root.querySelector<HTMLElement>("#cloud-profile-wrap")!;
  const localSelect = root.querySelector<HTMLSelectElement>("#cloud-local")!;
  const writeBtn = root.querySelector<HTMLButtonElement>("#cloud-write")!;
  const countEl = root.querySelector<HTMLElement>("#cloud-count")!;
  const body = root.querySelector<HTMLElement>("#cloud-body")!;

  // ---------- Dados ----------

  function localBinding(accountId: string, r: CloudRepo): string | null {
    if (r.local_binding_id) return r.local_binding_id;
    const name = r.full_name.toLowerCase();
    const b = bindings.find((x) => x.account_id === accountId && x.github_owner && x.github_repo && `${x.github_owner}/${x.github_repo}`.toLowerCase() === name);
    return b?.id ?? null;
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
      ctx.notifyDataChanged();
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
    localSelect.value = localFilter;
    writeBtn.setAttribute("aria-pressed", String(permFilter === "write"));
    profileWrap.hidden = accounts.length < 2;
    profileSelect.innerHTML = [`<option value="all">Todos os perfis</option>`, ...accounts.map((a) => `<option value="${esc(a.id)}">${esc(profileName(a))}</option>`)].join("");
    profileSelect.value = profileFilter;
  }

  function renderAll(): void {
    renderToolbar();
    renderBody();
  }

  function progressHtml(job: CloneJob): string {
    const pct = overallPercent(job);
    const label = PHASE_LABEL[job.phase] ?? "Clonando…";
    return `<div class="cloud-progress-wrap" data-clone-key="${esc(rowKey(job.accountId, job.fullName))}">
      <div class="cloud-progress" role="progressbar" aria-label="Clonando ${esc(job.fullName)}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct}">
        <span class="cloud-progress-fill" style="width:${pct}%"></span>
      </div>
      <span class="cloud-progress-label caption muted"><span class="cloud-progress-phase">${esc(label)}</span> · <span class="cloud-progress-pct">${pct}%</span></span>
    </div>`;
  }

  function popoverHtml(p: Popover, account: AccountRow): string {
    return `<div class="cloud-popover" role="dialog" aria-label="Clonar ${esc(p.fullName)}">
      <div class="field-label">Destino</div>
      <div class="cloud-dest" title="${esc(destLabel(p))}">${icon("folder")}<code class="truncate">${esc(destLabel(p))}</code></div>
      <div class="cloud-popover-choose">
        <button type="button" class="control" data-pop-choose>${icon("folder")}<span>Escolher pasta…</span></button>
        ${p.parent ? `<button type="button" class="control ghost" data-pop-reset>Usar a pasta padrão</button>` : ""}
      </div>
      <p class="caption muted">O GitPass clona com o perfil ${esc(profileName(account))} e vincula a pasta a ele.</p>
      <div class="cloud-popover-actions">
        <button type="button" class="control ghost" data-pop-cancel>Cancelar</button>
        <button type="button" class="control primary" data-pop-confirm>${icon("download")}<span>Clonar</span></button>
      </div>
    </div>`;
  }

  function repoRow(e: Entry): string {
    const { account: a, repo: r, local } = e;
    const key = rowKey(a.id, r.full_name);
    const job = activeClones.get(key);
    const fam = profileFamily(a.id);
    const tags = [
      r.private ? `<span class="cloud-tag">${icon("lock", 11)}Privado</span>` : "",
      r.archived ? `<span class="cloud-tag warn" title="Repositório arquivado (só leitura)">${icon("alert", 11)}Arquivado</span>` : "",
      r.fork ? `<span class="cloud-tag" title="Fork de outro repositório">${icon("branch", 11)}Fork</span>` : "",
      permissionTag(r),
    ].join("");
    const meta = [r.default_branch ? `${icon("branch", 11)}${esc(r.default_branch)}` : "", r.pushed_at ? `Último push ${esc(relativeTime(r.pushed_at))}` : ""]
      .filter(Boolean)
      .join(" · ");

    let action: string;
    if (job) {
      action = `<span class="state neutral" role="status"><span class="spinner" aria-hidden="true"></span>Clonando…</span>`;
    } else if (local) {
      action = `<button type="button" class="control" data-open-local${e.bindingId ? ` data-binding="${esc(e.bindingId)}" aria-controls="repo-drawer"` : ""}>Abrir${icon("chevronRight")}</button>`;
    } else {
      const open = popover?.key === key;
      action = `<button type="button" class="control" data-clone-open aria-haspopup="dialog" aria-expanded="${open}">${icon("plus")}<span>Clonar</span></button>`;
    }
    const pop = popover?.key === key && !job && !local ? popoverHtml(popover, a) : "";

    return `<li class="cloud-item" data-key="${esc(key)}" data-account="${esc(a.id)}" data-full="${esc(r.full_name)}">
      <div class="cloud-row">
        <span class="project-name"><span class="repo-mark">${icon("repo")}</span><span class="truncate">
          <strong class="truncate" title="${esc(r.description ?? r.name)}">${esc(r.name)}${local && !job ? `<span class="cloud-local" title="Clone local vinculado">${icon("check", 11)}Local</span>` : ""}</strong>
          <code class="truncate" title="${esc(r.full_name)}">${esc(r.full_name)}</code>
        </span></span>
        <span class="cloud-meta">${tags}${meta ? `<span class="cloud-sub">${meta}</span>` : ""}</span>
        ${profileChip(a)}
        <span class="cloud-actions">
          ${action}
          <button type="button" class="icon-button" data-open-url="${esc(r.html_url)}" aria-label="Abrir ${esc(r.full_name)} no GitHub" title="Abrir no GitHub">${icon("external")}</button>
        </span>
      </div>
      ${job ? progressHtml(job) : ""}
      ${pop}
    </li>`;
  }

  function noticeHtml(a: AccountRow): string {
    if (!a.has_token) {
      return `<div class="callout warning cloud-notice">${icon("alert")}<span><strong>${esc(profileName(a))}</strong> não está conectado ao GitHub, então não dá para listar os repositórios.</span>
        <button type="button" class="control" data-action="accounts">${icon("key")}<span>Conectar de novo</span></button></div>`;
    }
    const st = repoStates.get(a.id);
    if (st?.status === "error") {
      return errorView({
        title: `Não foi possível buscar os repositórios de ${profileName(a)}`,
        error: st.message,
        compact: true,
        tone: "warning",
        actions: [{ label: "Tentar de novo", action: "retry-profile", icon: "refresh", data: { retry: a.id } }],
      });
    }
    return "";
  }

  function entries(shown: AccountRow[]): { list: Entry[]; total: number } {
    const q = query.trim().toLowerCase();
    let total = 0;
    const list: Entry[] = [];
    for (const a of shown) {
      const st = repoStates.get(a.id);
      if (st?.status !== "ok") continue;
      total += st.repos.length;
      for (const r of st.repos) {
        const bindingId = localBinding(a.id, r);
        const local = !!bindingId || clonedKeys.has(rowKey(a.id, r.full_name));
        if (q && !r.full_name.toLowerCase().includes(q) && !(r.description ?? "").toLowerCase().includes(q)) continue;
        if (localFilter === "local" && !local) continue;
        if (localFilter === "cloud" && local) continue;
        if (permFilter === "write" && !canWrite(r)) continue;
        list.push({ account: a, repo: r, local, bindingId });
      }
    }
    list.sort((x, y) => Number(y.local) - Number(x.local) || (y.repo.pushed_at ?? "").localeCompare(x.repo.pushed_at ?? "") || x.repo.full_name.localeCompare(y.repo.full_name));
    return { list, total };
  }

  function renderBody(): void {
    if (!loadedAccounts) {
      countEl.textContent = "";
      body.innerHTML = `<div class="list-frame" aria-busy="true"><span class="sr-only">Carregando…</span>${'<div class="cloud-sk-row"><span class="skeleton"></span></div>'.repeat(4)}</div>`;
      return;
    }
    if (loadError) {
      body.innerHTML = errorView({
        title: "Não foi possível carregar os perfis",
        error: loadError,
        actions: [{ label: "Tentar de novo", action: "reload", icon: "refresh", primary: true }],
      });
      return;
    }
    if (accounts.length === 0) {
      countEl.textContent = "";
      body.innerHTML = `<section class="surface"><div class="empty-state">
        <span class="empty-icon">${icon("cloud")}</span>
        <h3>Nenhuma conta conectada</h3>
        <p>Conecte uma conta do GitHub para ver aqui os repositórios que você tem na nuvem.</p>
        <button type="button" class="control primary" data-action="accounts">${icon("plus")}<span>Conectar conta</span></button>
      </div></section>`;
      return;
    }
    const shown = accounts.filter((a) => profileFilter === "all" || a.id === profileFilter);
    const notices = shown.map(noticeHtml).join("");
    const loading = shown.filter((a) => repoStates.get(a.id)?.status === "loading");
    const { list, total } = entries(shown);
    const filtering = query.trim() !== "" || localFilter !== "all" || permFilter !== "all";
    countEl.textContent = filtering ? `${list.length} de ${total}` : plural(total, "repositório", "repositórios");
    const visible = showAll ? list : list.slice(0, PAGE_SIZE);
    const more = list.length - visible.length;
    const loadingNote = loading.length
      ? `<p class="cloud-loading caption muted" role="status"><span class="spinner" aria-hidden="true"></span>Buscando os repositórios de ${esc(loading.map(profileName).join(", "))}…</p>`
      : "";
    let listHtml: string;
    if (visible.length) {
      listHtml = `<ul class="list-frame cloud-list" aria-label="Repositórios na nuvem">${visible.map(repoRow).join("")}</ul>
        ${more > 0 ? `<div class="cloud-more"><button type="button" class="control" data-more>Mostrar mais (${more})</button></div>` : ""}`;
    } else if (loading.length && total === 0) {
      listHtml = `<div class="list-frame" aria-busy="true">${'<div class="cloud-sk-row"><span class="skeleton"></span></div>'.repeat(3)}</div>`;
    } else if (total === 0) {
      listHtml = notices ? "" : `<div class="list-frame empty"><strong>Nenhum repositório</strong>As contas conectadas ainda não têm repositórios no GitHub.</div>`;
    } else {
      listHtml = `<div class="list-frame empty"><strong>Nenhum repositório encontrado</strong>Experimente outra busca ou ajuste os filtros.</div>`;
    }
    body.innerHTML = `${notices}${loadingNote}${listHtml}`;
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
      if (phase) phase.textContent = PHASE_LABEL[job.phase] ?? "Clonando…";
      const pctEl = el.querySelector<HTMLElement>(".cloud-progress-pct");
      if (pctEl) pctEl.textContent = `${pct}%`;
    }
  }

  // ---------- Eventos ----------

  searchInput.addEventListener("input", () => {
    query = searchInput.value;
    renderBody();
  });
  localSelect.addEventListener("change", () => {
    localFilter = localSelect.value as LocalFilter;
    renderBody();
  });
  profileSelect.addEventListener("change", () => {
    profileFilter = profileSelect.value || "all";
    renderBody();
  });
  writeBtn.addEventListener("click", () => {
    permFilter = permFilter === "write" ? "all" : "write";
    writeBtn.setAttribute("aria-pressed", String(permFilter === "write"));
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
    if (el.closest("[data-more]")) {
      showAll = true;
      renderBody();
      return;
    }

    const url = el.closest<HTMLElement>("[data-open-url]")?.dataset.openUrl;
    if (url) {
      api.openExternal(url).catch((e) => showToast(errMessage(e)));
      return;
    }
    const openLocal = el.closest<HTMLElement>("[data-open-local]");
    if (openLocal) return ctx.navigate(openLocal.dataset.binding ? drawerRoute(openLocal.dataset.binding) : "repos");

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
      void withBusy(chooseBtn, "Escolhendo…", () => apiCloud.chooseFolder())
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
      const key = popover.key;
      popover = null;
      renderBody();
      body.querySelector<HTMLElement>(`.cloud-item[data-key="${CSS.escape(key)}"] [data-clone-open]`)?.focus();
    }
  };
  document.addEventListener("click", onDocClick);
  document.addEventListener("keydown", onKey);

  refreshBtn.addEventListener("click", () => {
    void withBusy(refreshBtn, "", loadAll).catch((e) => showToast(errMessage(e)));
  });

  ctx.onRoute((_p, q) => {
    const prof = q?.get("profile");
    if (prof && prof !== profileFilter) {
      profileFilter = prof;
      renderAll();
    }
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
