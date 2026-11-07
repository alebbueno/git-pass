import "./repos.css";
import { api, type AccountRow } from "../lib/api";
import { reposApi, type BindOutcome, type RepoRow, type SuggestionReason } from "../lib/api-repos";
import type { CurrentRepoState } from "../lib/api-system";
import { errMessage, esc, shortenPath, showToast, withBusy } from "../lib/dom";
import { errorView } from "../lib/errors";
import { icon } from "../lib/icons";
import { profileChip, profileName } from "../lib/profile";
import { onSystemState } from "../lib/system-state";
import { DATA_CHANGED_EVENT, DRAWER_EVENT, type DrawerEventDetail, type MountView } from "../lib/view";
import { drawerRoute } from "./repo-detail";
import { isPushing, onSessionChange, refreshOutgoing } from "./repo-push";
import { createDiscovery } from "./repos-discover";
import { startFolderMap } from "./repos-map";
import {
  REASON_TEXT,
  aheadOf,
  closeMenus,
  columnsHtml,
  openModal,
  plural,
  projectName,
  projectRowHtml,
  repoName,
  reviewReasons,
  whoText,
} from "./repos-ui";

type Filters = { profile: string; review: boolean; ahead: boolean; query: string };

const FILTERS_KEY = "gitpass:repos-filters";
const ALL_PROFILES = "";
const MISSING_PROFILE = "missing";

function loadFilters(): Filters {
  const base: Filters = { profile: ALL_PROFILES, review: false, ahead: false, query: "" };
  try {
    const raw = JSON.parse(sessionStorage.getItem(FILTERS_KEY) ?? "null") as Partial<Filters> | null;
    if (!raw || typeof raw !== "object") return base;
    return {
      profile: typeof raw.profile === "string" ? raw.profile : base.profile,
      review: raw.review === true,
      ahead: raw.ahead === true,
      query: typeof raw.query === "string" ? raw.query : "",
    };
  } catch {
    return base;
  }
}

function saveFilters(f: Filters): void {
  try {
    sessionStorage.setItem(FILTERS_KEY, JSON.stringify(f));
  } catch {
    // Sem sessionStorage os filtros só valem enquanto a tela está aberta.
  }
}

/** `repos?profile=<id>` e `repos?review=1` (links do Painel, de Contas e da barra lateral) trocam os filtros. */
function applyQuery(f: Filters, query: URLSearchParams | undefined): Filters {
  if (!query) return f;
  if (query.has("profile")) return { profile: query.get("profile") ?? "", review: false, ahead: false, query: "" };
  if (query.get("review") === "1") return { profile: ALL_PROFILES, review: true, ahead: false, query: "" };
  if (query.get("ahead") === "1") return { profile: ALL_PROFILES, review: false, ahead: true, query: "" };
  return f;
}

const normalize = (s: string): string => s.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();

/** Mantido entre montagens: rolagem da tela. */
const memory = { scroll: 0 };

export const mount: MountView = (root, ctx, _param, initialQuery) => {
  let accounts: AccountRow[] = [];
  let repos: RepoRow[] = [];
  let loaded = false;
  let loadToken = 0;
  let disposed = false;
  let filters = applyQuery(loadFilters(), initialQuery);
  let expanded: string | null = null;
  let reloadTimer: number | undefined;
  let system: CurrentRepoState | null = null;
  saveFilters(filters);

  root.classList.add("repos-view");
  const screen = root.closest<HTMLElement>(".screen");
  const who = (id: string): string => whoText(accounts, id);
  const openRepo = (id: string): void => ctx.navigate(drawerRoute(id));

  const discovery = createDiscovery({
    accounts: () => accounts,
    repos: () => repos,
    isDisposed: () => disposed,
    onUpdate: () => updateFoundCount(),
    onBound: (rows) => {
      rows.forEach(upsertRepo);
      ctx.notifyDataChanged();
      render();
    },
  });

  // ---------- Filtros ----------

  const needsReview = (b: RepoRow): boolean => reviewReasons(b, accounts, system).length > 0;
  const matchesProfile = (b: RepoRow, p: string): boolean =>
    p === ALL_PROFILES || (p === MISSING_PROFILE ? b.account_missing : !b.account_missing && b.account_id === p);
  const matchesQuery = (b: RepoRow, q: string): boolean => {
    const t = normalize(q.trim());
    if (!t) return true;
    const acc = accounts.find((a) => a.id === b.account_id);
    return normalize(
      [projectName(b), repoName(b), b.root_path, shortenPath(b.root_path), b.branch ?? "", b.remote_url ?? "", acc ? profileName(acc) : "", acc?.login ?? ""].join(" ")
    ).includes(t);
  };

  function sorted(list: RepoRow[]): RepoRow[] {
    return [...list].sort((a, b) => {
      const pa = aheadOf(a) > 0 ? 0 : 1;
      const pb = aheadOf(b) > 0 ? 0 : 1;
      return pa - pb || projectName(a).localeCompare(projectName(b), "pt-BR", { sensitivity: "base" });
    });
  }

  const visible = (): RepoRow[] =>
    sorted(
      repos.filter(
        (b) =>
          matchesProfile(b, filters.profile) &&
          (!filters.review || needsReview(b)) &&
          (!filters.ahead || aheadOf(b) > 0) &&
          matchesQuery(b, filters.query)
      )
    );

  function setFilters(next: Partial<Filters>): void {
    filters = { ...filters, ...next };
    saveFilters(filters);
    renderFilterbar();
    renderList();
  }

  function validateFilters(): void {
    const p = filters.profile;
    const ok = p === ALL_PROFILES || (p === MISSING_PROFILE && repos.some((b) => b.account_missing)) || accounts.some((a) => a.id === p);
    if (!ok) {
      filters.profile = ALL_PROFILES;
      saveFilters(filters);
    }
  }

  // ---------- Estrutura ----------

  function headActions(): string {
    if (!loaded) return "";
    if (accounts.length === 0) {
      return `<button type="button" class="control primary" data-action="go-accounts">${icon("plus")}<span>Conectar conta</span></button>`;
    }
    return `
      <button type="button" class="icon-button" data-action="refresh" aria-label="Atualizar a lista" title="Atualizar a lista">${icon("refresh")}</button>
      <button type="button" class="control" data-action="map">${icon("folder")}<span>Mapear pasta…</span></button>
      <button type="button" class="control" data-action="found" aria-controls="repos-discover">${icon("search")}<span>Encontrados neste Mac</span><span class="repos-found-count" data-ref="found-count"></span></button>
      <button type="button" class="control primary" data-action="bind">${icon("plus")}<span>Vincular repositório</span></button>`;
  }

  function renderShell(): void {
    root.innerHTML = `
      <div class="page-head">
        <div class="heading-group"><h1>Repositórios</h1><p>Gerencie a conta Git vinculada a cada projeto local.</p></div>
        <div class="page-actions" data-ref="actions">${headActions()}</div>
      </div>
      <div data-ref="body"></div>`;
  }

  const body = (): HTMLElement => root.querySelector<HTMLElement>('[data-ref="body"]')!;

  function renderActions(): void {
    const box = root.querySelector<HTMLElement>('[data-ref="actions"]');
    if (!box) return;
    const focusAction = box.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.action : undefined;
    box.innerHTML = headActions();
    if (focusAction) box.querySelector<HTMLElement>(`[data-action="${focusAction}"]`)?.focus();
    updateFoundCount();
  }

  function updateFoundCount(): void {
    const el = root.querySelector<HTMLElement>('[data-ref="found-count"]');
    if (!el) return;
    const n = discovery.count();
    el.textContent = n > 0 ? String(n) : "";
    el.hidden = n === 0;
  }

  function renderLoading(): void {
    body().innerHTML = `
      <div class="filterbar repos-filterbar-skeleton" aria-hidden="true"><div class="skeleton repos-skeleton-search"></div></div>
      <div class="list-frame" aria-busy="true">${columnsHtml()}
        <span class="sr-only">Carregando repositórios…</span>
        <ul class="projects">${Array.from({ length: 5 }, () => `<li class="repos-skeleton-row"><span class="skeleton"></span></li>`).join("")}</ul>
      </div>`;
  }

  function renderError(message: string): void {
    body().innerHTML = errorView({
      title: "Não foi possível carregar os repositórios",
      error: message,
      actions: [{ label: "Tentar de novo", action: "retry", icon: "refresh", primary: true }],
    });
  }

  function renderEmpty(): void {
    const noAccounts = accounts.length === 0;
    body().innerHTML = `
      <section class="surface">
        <div class="empty-state">
          <div class="empty-icon">${icon(noAccounts ? "user" : "repo")}</div>
          <h3>${noAccounts ? "Conecte primeiro uma conta do GitHub" : "Nenhum repositório vinculado"}</h3>
          <p>${
            noAccounts
              ? "Depois de conectar uma conta, o GitPass encontra seus clones e usa a conta certa em cada um."
              : "Mapeie a pasta onde ficam seus projetos: o GitPass encontra todos e sugere a conta certa para cada um. Depois disso, você confere e envia os commits por aqui."
          }</p>
          <div class="row-actions">${
            noAccounts
              ? `<button type="button" class="control primary" data-action="go-accounts">${icon("plus")}<span>Conectar conta</span></button>`
              : `<button type="button" class="control primary" data-action="map">${icon("folder")}<span>Mapear pasta…</span></button>
                 <button type="button" class="control" data-action="bind">${icon("plus")}<span>Vincular repositório</span></button>`
          }</div>
        </div>
      </section>
      ${noAccounts ? "" : `<section class="surface repos-discover" id="repos-discover" aria-labelledby="repos-discover-title"></section>`}`;
    discovery.render(body().querySelector<HTMLElement>(".repos-discover"), true);
    updateFoundCount();
  }

  function ensureLayout(): void {
    if (body().querySelector(".filterbar[data-ready]")) return;
    const mac = /Mac/i.test(navigator.platform);
    body().innerHTML = `
      <div class="filterbar" data-ready aria-label="Filtros dos repositórios" role="search">
        <label class="search">${icon("search")}<span class="sr-only">Buscar repositório</span>
          <input type="search" id="repo-search" placeholder="Buscar repositório" title="Busque por nome, pasta, branch ou perfil" autocomplete="off" spellcheck="false" />
          <kbd aria-hidden="true">${mac ? "⌘ K" : "Ctrl K"}</kbd>
        </label>
        <label class="select-wrap">${icon("user")}<span class="sr-only">Filtrar por perfil</span><select id="repo-profile"></select></label>
        <button type="button" class="control filter-review" data-filter="review" aria-pressed="false">${icon("alert")}<span>Para revisar</span></button>
        <button type="button" class="control filter-toggle" data-filter="ahead" aria-pressed="false">${icon("arrowUp")}<span>Para enviar</span></button>
        <span class="filter-count" role="status" aria-live="polite" data-ref="count"></span>
      </div>
      <div data-ref="list"></div>
      <div class="list-note"><span>${icon("info")}Selecione um repositório para abrir os detalhes.</span></div>
      <section class="surface repos-discover" id="repos-discover" aria-labelledby="repos-discover-title"></section>`;

    const input = body().querySelector<HTMLInputElement>("#repo-search")!;
    input.value = filters.query;
    input.addEventListener("input", () => setFilters({ query: input.value }));
    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        focusRow(null);
      } else if (e.key === "Escape" && input.value) {
        e.preventDefault();
        input.value = "";
        setFilters({ query: "" });
      }
    });
    body()
      .querySelector<HTMLSelectElement>("#repo-profile")!
      .addEventListener("change", (e) => setFilters({ profile: (e.target as HTMLSelectElement).value }));
    const list = body().querySelector<HTMLElement>('[data-ref="list"]')!;
    list.addEventListener("click", (e) => {
      const row = (e.target as HTMLElement).closest<HTMLButtonElement>(".project-row");
      if (row?.dataset.repoId) openRepo(row.dataset.repoId);
    });
    list.addEventListener("keydown", onListKey);
  }

  function renderFilterbar(): void {
    const select = body().querySelector<HTMLSelectElement>("#repo-profile");
    if (!select) return;
    const options = [
      `<option value="${ALL_PROFILES}">Todos os perfis</option>`,
      ...accounts
        .filter((a) => repos.some((b) => matchesProfile(b, a.id)) || filters.profile === a.id)
        .map((a) => `<option value="${esc(a.id)}"${filters.profile === a.id ? " selected" : ""}>${esc(profileName(a))}</option>`),
      ...(repos.some((b) => b.account_missing)
        ? [`<option value="${MISSING_PROFILE}"${filters.profile === MISSING_PROFILE ? " selected" : ""}>Conta removida</option>`]
        : []),
    ];
    select.innerHTML = options.join("");
    select.value = filters.profile;
    body().querySelector('[data-filter="review"]')?.setAttribute("aria-pressed", String(filters.review));
    body().querySelector('[data-filter="ahead"]')?.setAttribute("aria-pressed", String(filters.ahead));
    const input = body().querySelector<HTMLInputElement>("#repo-search");
    if (input && document.activeElement !== input && input.value !== filters.query) input.value = filters.query;
  }

  function renderList(): void {
    const list = body().querySelector<HTMLElement>('[data-ref="list"]');
    if (!list) return;
    const focusedId = list.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.repoId : undefined;
    const items = visible();
    const count = body().querySelector<HTMLElement>('[data-ref="count"]');
    if (count) count.textContent = plural(items.length, "repositório", "repositórios");
    if (items.length === 0) {
      list.innerHTML = `<div class="list-frame empty"><strong>Nenhum repositório encontrado</strong>Experimente outra busca ou ajuste os filtros.
        <div><button type="button" class="control" data-action="clear-filters">${icon("x")}<span>Limpar filtros</span></button></div></div>`;
      return;
    }
    const anchor = items.find((b) => b.id === (focusedId ?? expanded))?.id ?? items[0].id;
    list.innerHTML = `<div class="list-frame">${columnsHtml()}<ul class="projects" aria-label="Repositórios locais">${items
      .map((b) => projectRowHtml(b, accounts, system, { expanded: b.id === expanded, tabbable: b.id === anchor }))
      .join("")}</ul></div>`;
    if (focusedId) list.querySelector<HTMLElement>(`.project-row[data-repo-id="${CSS.escape(focusedId)}"]`)?.focus({ preventScroll: true });
  }

  function rows(): HTMLButtonElement[] {
    return Array.from(body().querySelectorAll<HTMLButtonElement>(".projects .project-row"));
  }

  function focusRow(id: string | null): void {
    const all = rows();
    const target = (id ? all.find((r) => r.dataset.repoId === id) : undefined) ?? all.find((r) => r.tabIndex === 0) ?? all[0];
    if (!target) return;
    all.forEach((r) => (r.tabIndex = r === target ? 0 : -1));
    target.focus();
    target.scrollIntoView({ block: "nearest" });
  }

  function onListKey(e: KeyboardEvent): void {
    const cur = (e.target as HTMLElement).closest<HTMLButtonElement>(".project-row");
    if (!cur) return;
    const all = rows();
    let i = all.indexOf(cur);
    switch (e.key) {
      case "ArrowDown":
        i = Math.min(all.length - 1, i + 1);
        break;
      case "ArrowUp":
        if (i <= 0) {
          e.preventDefault();
          body().querySelector<HTMLInputElement>("#repo-search")?.focus();
          return;
        }
        i -= 1;
        break;
      case "Home":
        i = 0;
        break;
      case "End":
        i = all.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    focusRow(all[i]?.dataset.repoId ?? null);
  }

  function render(): void {
    if (disposed) return;
    closeMenus();
    renderActions();
    if (repos.length === 0) {
      renderEmpty();
      return;
    }
    ensureLayout();
    renderFilterbar();
    renderList();
    discovery.render(body().querySelector<HTMLElement>(".repos-discover"), false);
    updateFoundCount();
  }

  // ---------- Vincular ----------

  function upsertRepo(row: RepoRow): void {
    const i = repos.findIndex((r) => r.id === row.id);
    if (i >= 0) repos[i] = row;
    else repos.push(row);
  }

  function afterBound(row: RepoRow, reason: SuggestionReason | null): void {
    upsertRepo(row);
    discovery.forget(row.root_path);
    const why = reason ? ` (${REASON_TEXT[reason]})` : "";
    showToast(`${repoName(row)} vinculado a ${who(row.account_id)}${why}.`, "ok");
    ctx.notifyDataChanged();
    render();
    openRepo(row.id);
  }

  async function bindWithDialog(btn: HTMLButtonElement): Promise<void> {
    let outcome: BindOutcome;
    try {
      outcome = await withBusy(btn, "Aguardando a pasta…", () => reposApi.bindDialog(null));
    } catch (e) {
      showToast(errMessage(e) || "Não foi possível vincular o repositório.", "err");
      return;
    }
    if (disposed || outcome.status === "cancelled") return;
    if (outcome.status === "bound") {
      afterBound(outcome.repo, outcome.reason);
      return;
    }
    const accountId = await confirmAccount(outcome);
    if (!accountId || disposed) return;
    try {
      const row = await withBusy(btn, "Vinculando…", () => reposApi.bindPath(outcome.root_path, accountId));
      if (!disposed) afterBound(row, null);
    } catch (e) {
      showToast(errMessage(e) || "Não foi possível vincular o repositório.", "err");
    }
  }

  function confirmAccount(o: Extract<BindOutcome, { status: "needs_account" }>): Promise<string | null> {
    const pushText = (v: boolean | null | undefined): string =>
      v === true
        ? `<span class="badge success">${icon("check", 12)}Pode enviar</span>`
        : v === false
          ? `<span class="badge warning">${icon("alert", 12)}Sem permissão para enviar</span>`
          : `<span class="badge">${icon("help", 12)}Não confirmado</span>`;
    const preselected = o.suggested_account_id ?? accounts[0]?.id;
    const bodyHtml = `
      <p class="repos-modal-lead">Qual conta deve ser usada em <span class="mono">${esc(`${o.github_owner}/${o.github_repo}`)}</span>?</p>
      <p class="muted small repos-modal-path mono" title="${esc(o.root_path)}">${esc(shortenPath(o.root_path))}</p>
      <fieldset class="repos-pick">
        <legend class="sr-only">Conta</legend>
        ${accounts
          .map((a) => {
            const c = o.candidates.find((x) => x.account_id === a.id);
            return `<label class="list-row repos-pick-row">
              <input type="radio" name="repos-pick" value="${esc(a.id)}"${a.id === preselected ? " checked" : ""} />
              ${profileChip(a)}
              ${pushText(c?.can_push)}
            </label>`;
          })
          .join("")}
      </fieldset>`;
    return openModal<string>({
      title: "Escolher a conta",
      body: bodyHtml,
      confirmLabel: "Vincular",
      read: (d) => d.querySelector<HTMLInputElement>('input[name="repos-pick"]:checked')?.value,
    });
  }

  function openFound(): void {
    const sec = body().querySelector<HTMLElement>(".repos-discover");
    if (!sec) return;
    discovery.open();
    sec.scrollIntoView({ block: "start", behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    sec.querySelector<HTMLElement>("summary")?.focus({ preventScroll: true });
  }

  // ---------- Eventos ----------

  root.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    const filterBtn = target.closest<HTMLButtonElement>("button[data-filter]");
    if (filterBtn) {
      if (filterBtn.dataset.filter === "review") setFilters({ review: !filters.review });
      else if (filterBtn.dataset.filter === "ahead") setFilters({ ahead: !filters.ahead });
      return;
    }
    const btn = target.closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled || btn.closest(".repos-discover")) return;
    switch (btn.dataset.action) {
      case "map":
        void startFolderMap(ctx, btn);
        break;
      case "bind":
        void bindWithDialog(btn);
        break;
      case "found":
        openFound();
        break;
      case "go-accounts":
        ctx.navigate("accounts");
        break;
      case "refresh":
        void withBusy(btn, "", async () => {
          await load();
          if (expanded && !isPushing(expanded)) await refreshOutgoing(expanded, false);
        });
        break;
      case "retry":
        void load();
        break;
      case "clear-filters":
        setFilters({ profile: ALL_PROFILES, review: false, ahead: false, query: "" });
        body().querySelector<HTMLInputElement>("#repo-search")?.focus();
        break;
    }
  });

  async function load(): Promise<void> {
    const token = ++loadToken;
    if (!loaded) renderLoading();
    try {
      const [accs, list] = await Promise.all([api.accounts(), reposApi.list()]);
      if (disposed || token !== loadToken) return;
      accounts = accs;
      repos = list;
      const first = !loaded;
      loaded = true;
      validateFilters();
      render();
      discovery.autoRun();
      if (first && screen && memory.scroll) screen.scrollTop = memory.scroll;
    } catch (e) {
      if (disposed || token !== loadToken) return;
      if (loaded) showToast(`Não foi possível atualizar os repositórios: ${errMessage(e)}`, "err");
      else {
        renderActions();
        renderError(errMessage(e));
      }
    }
  }

  /** Vários avisos seguidos (push, correção de autor, config) viram um só recarregamento. */
  function scheduleReload(): void {
    window.clearTimeout(reloadTimer);
    reloadTimer = window.setTimeout(() => void load(), 250);
  }

  ctx.onRoute((_param, query) => {
    if (disposed) return;
    const next = applyQuery(filters, query);
    if (next !== filters) {
      filters = next;
      saveFilters(filters);
      if (loaded) render();
    }
  });
  const onDataChanged = (): void => scheduleReload();
  document.addEventListener(DATA_CHANGED_EVENT, onDataChanged);
  const onDrawer = (e: Event): void => {
    expanded = (e as CustomEvent<DrawerEventDetail>).detail.id;
    rows().forEach((r) => r.setAttribute("aria-expanded", String(r.dataset.repoId === expanded)));
  };
  document.addEventListener(DRAWER_EVENT, onDrawer);
  const offSession = onSessionChange((_id, what) => {
    if (what === "pushed") scheduleReload();
  });
  let unlistenOutgoing: (() => void) | null = null;
  reposApi
    .onOutgoingChanged(() => {
      if (!disposed) scheduleReload();
    })
    .then((un) => {
      if (disposed) un();
      else unlistenOutgoing = un;
    })
    .catch(() => {});
  const onScroll = (): void => {
    memory.scroll = screen?.scrollTop ?? 0;
  };
  screen?.addEventListener("scroll", onScroll, { passive: true });

  const offSystem = onSystemState((s) => {
    const next = s.current.status === "ok" ? s.current.value : null;
    if (next === system) return;
    system = next;
    if (loaded && !disposed && repos.length) renderList();
  });

  renderShell();
  void load();

  return () => {
    disposed = true;
    offSystem();
    window.clearTimeout(reloadTimer);
    closeMenus();
    discovery.dispose();
    offSession();
    unlistenOutgoing?.();
    screen?.removeEventListener("scroll", onScroll);
    document.removeEventListener(DATA_CHANGED_EVENT, onDataChanged);
    document.removeEventListener(DRAWER_EVENT, onDrawer);
    root.classList.remove("repos-view");
  };
};
