import "./accounts.css";
import { api, type OauthConfigStatus } from "../lib/api";
import { accountsApi, type AccountCheck, type AccountEmails, type AccountInfo } from "../lib/api-accounts";
import { errMessage, esc, showToast, withBusy } from "../lib/dom";
import { DeviceFlowModal, ModalBase } from "../lib/device-flow";
import { icon } from "../lib/icons";
import { profileAvatar, profileFamily } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView, type ViewContext } from "../lib/view";

type CheckState = { state: "checking" } | AccountCheck;

type EmailMenu = {
  accountId: string;
  loading: boolean;
  data: AccountEmails | null;
  error: string | null;
  saving: string | null;
};

type State = {
  loading: boolean;
  error: string | null;
  accounts: AccountInfo[];
  oauth: OauthConfigStatus | null;
  checks: Map<string, CheckState>;
  editing: { id: string; draft: string; saving: boolean } | null;
  emailMenu: EmailMenu | null;
};

const LABEL_MAX = 64;
const OAUTH_UNAVAILABLE = "A conexão com o GitHub não está disponível nesta versão do app.";
const GITHUB_APPS_URL = "https://github.com/settings/applications";

/** Permissões OAuth mostradas no cartão; `required` sinaliza quando faltam. */
const SCOPES: { scope: string; label: string; hint: string; required: boolean; impliedBy: string[] }[] = [
  { scope: "repo", label: "Repositórios", hint: "Ler e enviar código (push) nos seus repositórios", required: true, impliedBy: [] },
  { scope: "user:email", label: "Email", hint: "Ler os emails verificados para usar nos commits", required: true, impliedBy: ["user"] },
  { scope: "workflow", label: "Workflows", hint: "Enviar alterações em arquivos do GitHub Actions", required: false, impliedBy: [] },
  { scope: "read:org", label: "Organizações", hint: "Ver os repositórios das organizações das quais você participa", required: false, impliedBy: ["write:org", "admin:org"] },
];

const svg = (d: string, size = 16) =>
  `<svg class="icon" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
const pencilIcon = (size = 14) => svg('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>', size);
const trashIcon = (size = 16) => svg('<path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>', size);
const chevronIcon = (size = 14) => svg('<path d="m6 9 6 6 6-6"/>', size);

export const mount: MountView = (root, ctx) => {
  const state: State = {
    loading: true,
    error: null,
    accounts: [],
    oauth: null,
    checks: new Map(),
    editing: null,
    emailMenu: null,
  };
  const failedAvatars = new Set<string>();
  let loadSeq = 0;
  let disposed = false;
  /** Verdadeiro enquanto o DOM é substituído: o `focusout` do campo de rótulo não conta como "sair do campo". */
  let rendering = false;
  let modal: { close(): void } | null = null;

  root.innerHTML = `
    <header class="view-header">
      <div>
        <h1 class="view-title">Contas</h1>
        <p class="view-sub" data-ref="sub">Carregando…</p>
      </div>
      <div class="view-actions">
        <button type="button" class="btn primary" data-action="connect" hidden>
          ${icon("plus")}<span>Conectar conta GitHub</span>
        </button>
      </div>
    </header>
    <p class="sr-only" data-ref="live" role="status" aria-live="polite"></p>
    <div class="accounts-body" data-ref="body" aria-busy="true"></div>
  `;

  const sub = root.querySelector<HTMLElement>('[data-ref="sub"]')!;
  const live = root.querySelector<HTMLElement>('[data-ref="live"]')!;
  const body = root.querySelector<HTMLElement>('[data-ref="body"]')!;
  const headerConnect = root.querySelector<HTMLButtonElement>('[data-action="connect"]')!;

  const oauthReady = () => state.oauth?.ok === true;
  const findAccount = (id: string | undefined) => state.accounts.find((a) => a.id === id);
  const announce = (msg: string) => {
    live.textContent = "";
    window.setTimeout(() => (live.textContent = msg), 50);
  };

  // ---------- Dados ----------

  async function load(): Promise<void> {
    const seq = ++loadSeq;
    state.loading = true;
    state.error = null;
    if (!state.accounts.length) render();
    const [oauthRes, accRes] = await Promise.allSettled([api.oauthStatus(), accountsApi.list()]);
    if (disposed || seq !== loadSeq) return;
    state.loading = false;
    state.oauth = oauthRes.status === "fulfilled" ? oauthRes.value : null;
    if (accRes.status === "fulfilled") {
      state.accounts = accRes.value;
      const ids = new Set(state.accounts.map((a) => a.id));
      for (const id of [...state.checks.keys()]) if (!ids.has(id)) state.checks.delete(id);
      if (state.editing && !ids.has(state.editing.id)) state.editing = null;
      if (state.emailMenu && !ids.has(state.emailMenu.accountId)) state.emailMenu = null;
    } else {
      state.error = errMessage(accRes.reason);
    }
    render();
    for (const a of state.accounts) if (a.has_token && !state.checks.has(a.id)) void runCheck(a.id);
  }

  async function runCheck(id: string): Promise<void> {
    state.checks.set(id, { state: "checking" });
    updateCard(id);
    let res: CheckState;
    try {
      res = await accountsApi.check(id);
    } catch (e) {
      res = { account_id: id, state: "error", scopes: [], missing_scopes: [], needs_reconnect: false, message: errMessage(e) };
    }
    if (disposed || !findAccount(id)) return;
    state.checks.set(id, res);
    updateCard(id);
  }

  function replaceAccount(row: AccountInfo): void {
    state.accounts = state.accounts.map((a) => (a.id === row.id ? row : a));
  }

  // ---------- Render ----------

  /** Re-render sem perder o foco do teclado: o elemento focado é reencontrado pela mesma chave. */
  function keepFocus(fn: () => void): void {
    const active = document.activeElement as HTMLElement | null;
    const key = active && root.contains(active) ? focusKey(active) : null;
    rendering = true;
    try {
      fn();
    } finally {
      rendering = false;
    }
    if (!key) return;
    const next = [...root.querySelectorAll<HTMLElement>("[data-action]")].find((el) => focusKey(el) === key);
    if (next) {
      next.focus();
      if (next instanceof HTMLInputElement) next.setSelectionRange(next.value.length, next.value.length);
    }
  }

  function render(): void {
    body.setAttribute("aria-busy", String(state.loading));
    const n = state.accounts.length;
    const ready = oauthReady();
    headerConnect.hidden = !ready;

    if (state.loading && !n) {
      sub.textContent = "Carregando…";
      body.innerHTML = renderSkeleton();
      return;
    }

    sub.textContent = n === 0 ? "Nenhuma conta conectada" : n === 1 ? "1 conta conectada" : `${n} contas conectadas`;

    let html = ready ? "" : renderOauthUnavailable();
    if (state.error) {
      html += `
        <div class="callout error accounts-error" role="alert">
          ${icon("alert")}
          <div class="accounts-callout-body">
            <strong>Não foi possível carregar as contas.</strong>
            <span>${esc(state.error)}</span>
            <div><button type="button" class="btn" data-action="reload">${icon("refresh")}<span>Tentar de novo</span></button></div>
          </div>
        </div>`;
    } else if (n === 0) {
      html += renderEmpty(ready);
    } else {
      html += `<div class="accounts-grid">${state.accounts.map((a) => renderCard(a)).join("")}</div>`;
    }
    keepFocus(() => (body.innerHTML = html));
  }

  function updateCard(id: string): void {
    const el = body.querySelector<HTMLElement>(`[data-card="${CSS.escape(id)}"]`);
    const a = findAccount(id);
    if (!el || !a) return;
    keepFocus(() => (el.outerHTML = renderCard(a)));
  }

  function renderOauthUnavailable(): string {
    return `
      <div class="callout warning accounts-oauth-callout" role="status">
        ${icon("alert")}
        <div class="accounts-callout-body"><strong>${esc(OAUTH_UNAVAILABLE)}</strong></div>
      </div>`;
  }

  function renderEmpty(ready: boolean): string {
    return `
      <section class="panel accounts-empty">
        <div class="empty-state">
          <div class="empty-icon">${icon("user", 28)}</div>
          <h3>Nenhuma conta conectada</h3>
          ${
            ready
              ? `<p>Conecte uma conta GitHub para vincular repositórios a um perfil. O GitPass mostra um código curto. Você abre o GitHub no navegador, digita o código e autoriza. O acesso fica salvo nas Chaves do macOS (Keychain).</p>
                 <button type="button" class="btn primary lg" data-action="connect">${icon("plus")}<span>Conectar conta GitHub</span></button>`
              : `<p>Quando a conexão com o GitHub estiver disponível, você poderá conectar suas contas aqui.</p>`
          }
        </div>
      </section>`;
  }

  function renderCard(a: AccountInfo): string {
    const fam = profileFamily(a.id);
    const titleId = `accounts-card-${esc(a.id)}`;
    const check = state.checks.get(a.id);
    const repoText = a.repos_count === 0 ? "Sem repositórios locais" : a.repos_count === 1 ? "1 repositório local" : `${a.repos_count} repositórios locais`;
    const revoked = !a.has_token || (check && check.state !== "checking" && (check.state === "revoked" || check.state === "missing"));

    return `
      <article class="panel accounts-card profile-${fam}" data-card="${esc(a.id)}" aria-labelledby="${titleId}">
        <div class="accounts-card-head">
          ${renderAvatar(a)}
          <div class="accounts-card-id">${renderIdentity(a, titleId)}</div>
        </div>
        <dl class="accounts-facts">
          <div class="accounts-fact">
            <dt>Email de commit</dt>
            <dd>${renderEmail(a, !!revoked)}</dd>
          </div>
          <div class="accounts-fact">
            <dt>Permissões</dt>
            <dd class="accounts-badges">${renderScopes(a, check)}</dd>
          </div>
          <div class="accounts-fact">
            <dt>Situação</dt>
            <dd class="accounts-badges">${renderTokenState(a, check)}</dd>
          </div>
        </dl>
        ${state.emailMenu?.accountId === a.id ? renderEmailMenu(a, state.emailMenu) : ""}
        ${renderAttention(a, check)}
        <div class="accounts-card-foot">
          <span class="accounts-repo-count small">${icon("folder", 14)}<span>${esc(repoText)}</span></span>
          <div class="accounts-card-actions">
            ${a.repos_count ? `<button type="button" class="btn ghost" data-action="repos">${icon("repo")}<span>Ver repositórios</span></button>` : ""}
            ${
              a.has_token && !revoked
                ? `<button type="button" class="btn ghost icon-only" data-action="refresh" data-account-id="${esc(a.id)}" title="Atualizar nome, email e foto com os dados do GitHub" aria-label="Atualizar perfil de ${esc(handle(a))}">${icon("refresh")}</button>`
                : ""
            }
            <button type="button" class="btn ghost icon-only accounts-danger" data-action="remove" data-account-id="${esc(a.id)}" title="Remover conta" aria-label="Remover conta ${esc(handle(a))}">${trashIcon()}</button>
          </div>
        </div>
      </article>`;
  }

  function renderAvatar(a: AccountInfo): string {
    const fallback = profileAvatar({ id: a.id, login: a.login, label: a.label || a.name }, 48);
    const src = a.avatar_url ? sizedAvatar(a.avatar_url, 96) : null;
    const img =
      src && !failedAvatars.has(src)
        ? `<img class="accounts-avatar-img" src="${esc(src)}" alt="" width="48" height="48" decoding="async" referrerpolicy="no-referrer" />`
        : "";
    return `<span class="accounts-avatar">${fallback}${img}</span>`;
  }

  function renderIdentity(a: AccountInfo, titleId: string): string {
    const title = displayName(a);
    const secondary = [a.login ? `@${a.login}` : null, a.label && a.name && a.name !== a.label ? a.name : null].filter(Boolean).join(" · ");
    const subLine = `<p class="muted small truncate accounts-handle">${secondary ? esc(secondary) : "sem login"}</p>`;
    const ed = state.editing;
    if (ed?.id === a.id) {
      return `
        <label class="sr-only" for="accounts-label-${esc(a.id)}">Rótulo da conta ${esc(handle(a))}</label>
        <input id="accounts-label-${esc(a.id)}" class="input accounts-label-input" data-action="label-input" data-account-id="${esc(a.id)}"
          value="${esc(ed.draft)}" maxlength="${LABEL_MAX}" placeholder="${esc(a.name || a.login || "Rótulo")}"
          autocomplete="off" spellcheck="false" aria-describedby="accounts-label-hint-${esc(a.id)}" ${ed.saving ? "disabled" : ""} />
        <p class="muted small accounts-label-hint" id="accounts-label-hint-${esc(a.id)}">${
          ed.saving ? "Salvando…" : "Enter salva · Esc cancela · deixar vazio volta ao nome do GitHub"
        }</p>
        <h3 class="sr-only" id="${titleId}">${esc(title)}</h3>`;
    }
    return `
      <div class="accounts-title-row">
        <h3 class="panel-title truncate" id="${titleId}">${esc(title)}</h3>
        <button type="button" class="btn ghost icon-only accounts-edit-btn" data-action="edit-label" data-account-id="${esc(a.id)}" title="Editar rótulo" aria-label="Editar rótulo de ${esc(handle(a))}">${pencilIcon()}</button>
      </div>
      ${subLine}`;
  }

  function renderEmail(a: AccountInfo, revoked: boolean): string {
    const open = state.emailMenu?.accountId === a.id;
    const value = a.email
      ? `<span class="mono small truncate" title="${esc(a.email)}">${esc(a.email)}</span>${
          isNoreply(a.email) ? `<span class="badge" title="Endereço privado do GitHub: seu email pessoal não aparece nos commits">${icon("lock", 12)}Privado</span>` : ""
        }`
      : `<span class="muted small">Não definido</span>`;
    const btn =
      a.has_token && !revoked
        ? `<button type="button" class="btn ghost accounts-email-btn" data-action="email-menu" data-account-id="${esc(a.id)}"
            aria-haspopup="menu" aria-expanded="${open}" aria-controls="accounts-email-menu-${esc(a.id)}" aria-label="Alterar email de commit de ${esc(handle(a))}">
            <span>Alterar</span>${chevronIcon()}
          </button>`
        : "";
    return `<span class="accounts-email">${value}</span>${btn}`;
  }

  function renderEmailMenu(a: AccountInfo, m: EmailMenu): string {
    let inner: string;
    if (m.loading) {
      inner = `<p class="accounts-menu-status small muted" role="status"><span class="spinner" aria-hidden="true"></span>Buscando emails verificados…</p>`;
    } else if (m.error) {
      inner = `
        <p class="accounts-menu-status small err" role="alert">${icon("alert", 14)}<span>${esc(m.error)}</span></p>
        <div><button type="button" class="btn" data-action="email-menu-retry" data-account-id="${esc(a.id)}">${icon("refresh")}<span>Tentar de novo</span></button></div>`;
    } else {
      const opts = m.data?.emails ?? [];
      inner = `
        <div class="accounts-menu-list" role="menu" aria-label="Emails verificados de ${esc(handle(a))}">
          ${opts
            .map((o) => {
              const tag = o.primary ? "Principal" : o.noreply ? "Privado (noreply)" : "";
              const busy = m.saving === o.email;
              return `
                <button type="button" class="accounts-menu-item" role="menuitemradio" aria-checked="${o.selected}"
                  data-action="email-pick" data-account-id="${esc(a.id)}" data-email="${esc(o.email)}" ${m.saving ? "disabled" : ""}>
                  <span class="accounts-menu-check" aria-hidden="true">${busy ? '<span class="spinner"></span>' : o.selected ? icon("check", 14) : ""}</span>
                  <span class="mono small truncate">${esc(o.email)}</span>
                  ${tag ? `<span class="badge">${esc(tag)}</span>` : ""}
                </button>`;
            })
            .join("")}
        </div>
        ${
          m.data?.needs_reconnect
            ? `<p class="small warn accounts-menu-note">${icon("alert", 14)}<span>Sem permissão de email: só aparecem o email público e o privado. Conecte a conta de novo para ver todos os emails verificados.</span></p>`
            : ""
        }
        <p class="small muted accounts-menu-note">Os repositórios vinculados a esta conta passam a usar o email escolhido nos próximos commits.</p>`;
    }
    return `
      <div class="accounts-email-menu" id="accounts-email-menu-${esc(a.id)}">
        <div class="accounts-email-menu-head">
          <span class="small">Escolher email de commit</span>
          <button type="button" class="btn ghost icon-only" data-action="email-menu-close" data-account-id="${esc(a.id)}" aria-label="Fechar lista de emails">${icon("x", 14)}</button>
        </div>
        ${inner}
      </div>`;
  }

  function renderScopes(a: AccountInfo, check: CheckState | undefined): string {
    if (!a.has_token) return `<span class="muted small">—</span>`;
    const live = check && check.state !== "checking" && check.scopes.length ? check.scopes : null;
    const scopes = live ?? a.scopes;
    if (!scopes.length) {
      return check?.state === "checking"
        ? `<span class="muted small">Verificando…</span>`
        : `<span class="muted small">Ainda não verificado</span>`;
    }
    const badges = SCOPES.map((s) => {
      const granted = scopes.includes(s.scope) || s.impliedBy.some((i) => scopes.includes(i));
      if (granted) return `<span class="badge" title="${esc(s.hint)}">${icon("check", 12)}${esc(s.label)}</span>`;
      if (s.required) return `<span class="badge warning" title="Faltando: ${esc(s.hint)}">${icon("alert", 12)}Sem ${esc(s.label.toLowerCase())}</span>`;
      return "";
    }).join("");
    return badges || `<span class="muted small">Sem permissões relevantes</span>`;
  }

  function renderTokenState(a: AccountInfo, check: CheckState | undefined): string {
    if (!a.has_token) return `<span class="badge warning">${icon("alert", 12)}Sem acesso</span>`;
    if (!check || check.state === "checking") {
      return `<span class="badge accounts-badge-checking"><span class="spinner" aria-hidden="true"></span>Verificando o acesso…</span>`;
    }
    const title = `title="${esc(check.message)}"`;
    switch (check.state) {
      case "ok":
        return `<span class="badge success" ${title}>${icon("check", 12)}Acesso válido</span>`;
      case "revoked":
        return `<span class="badge error" ${title}>${icon("x", 12)}Acesso revogado</span>`;
      case "missing":
        return `<span class="badge warning" ${title}>${icon("alert", 12)}Sem acesso</span>`;
      case "offline":
        return `<span class="badge" ${title}>${icon("cloud", 12)}Sem conexão</span>
          <button type="button" class="btn ghost accounts-inline-btn" data-action="recheck" data-account-id="${esc(a.id)}">Verificar</button>`;
      default:
        return `<span class="badge warning" ${title}>${icon("alert", 12)}Não verificado</span>
          <button type="button" class="btn ghost accounts-inline-btn" data-action="recheck" data-account-id="${esc(a.id)}">Verificar</button>`;
    }
  }

  function renderAttention(a: AccountInfo, check: CheckState | undefined): string {
    let message: string | null = null;
    if (!a.has_token) {
      message = "O acesso desta conta não está nas Chaves do macOS. Conecte a conta de novo para continuar usando este perfil.";
    } else if (check && check.state !== "checking" && check.needs_reconnect) {
      if (check.state === "ok" && check.missing_scopes.length) {
        const names = check.missing_scopes.map(scopeLabel).join(" e ");
        message = `Faltam permissões (${names}). Conecte a conta de novo para dar essas permissões ao GitPass.`;
      } else {
        message = check.message;
      }
    }
    if (!message) return "";
    const ready = oauthReady();
    return `
      <div class="accounts-card-warn" role="status">
        <span class="small">${esc(message)}</span>
        ${
          ready
            ? `<button type="button" class="btn" data-action="reconnect" data-account-id="${esc(a.id)}">${icon("refresh")}<span>Conectar de novo</span></button>`
            : ""
        }
      </div>`;
  }

  // ---------- Ações ----------

  function openFlow(opener: HTMLElement, account?: AccountInfo): void {
    if (modal || !oauthReady()) return;
    modal = new DeviceFlowModal(ctx, opener, account ?? null, (accountId) => {
      state.checks.delete(accountId);
    }, () => {
      modal = null;
    });
  }

  function startEdit(id: string): void {
    const a = findAccount(id);
    if (!a) return;
    state.emailMenu = null;
    state.editing = { id, draft: a.label ?? "", saving: false };
    updateCard(id);
    const input = body.querySelector<HTMLInputElement>(`[data-card="${CSS.escape(id)}"] [data-action="label-input"]`);
    input?.focus();
    input?.select();
  }

  /** `refocus` é falso quando o utilizador saiu do campo para outro controlo (não lhe roubamos o foco). */
  function cancelEdit(refocus = true): void {
    const ed = state.editing;
    if (!ed || ed.saving) return;
    state.editing = null;
    updateCard(ed.id);
    if (refocus) focusIn(ed.id, '[data-action="edit-label"]');
  }

  async function saveEdit(refocus = true): Promise<void> {
    const ed = state.editing;
    if (!ed || ed.saving) return;
    const a = findAccount(ed.id);
    const next = ed.draft.trim();
    if (!a || next === (a.label ?? "")) {
      cancelEdit(refocus);
      return;
    }
    ed.saving = true;
    updateCard(ed.id);
    try {
      const row = await accountsApi.setLabel(ed.id, next);
      if (disposed) return;
      replaceAccount(row);
      if (state.editing === ed) state.editing = null;
      updateCard(ed.id);
      if (refocus && !state.editing) focusIn(ed.id, '[data-action="edit-label"]');
      announce(next ? `Rótulo alterado para ${next}.` : "Rótulo removido.");
      ctx.notifyDataChanged();
    } catch (e) {
      if (disposed) return;
      ed.saving = false;
      updateCard(ed.id);
      if (state.editing === ed) focusIn(ed.id, '[data-action="label-input"]');
      showToast(errMessage(e));
    }
  }

  async function openEmailMenu(id: string): Promise<void> {
    state.editing = null;
    const m: EmailMenu = { accountId: id, loading: true, data: null, error: null, saving: null };
    const prev = state.emailMenu?.accountId;
    state.emailMenu = m;
    if (prev && prev !== id) updateCard(prev);
    updateCard(id);
    try {
      m.data = await accountsApi.emails(id);
    } catch (e) {
      m.error = errMessage(e);
    }
    m.loading = false;
    if (disposed || state.emailMenu !== m) return;
    updateCard(id);
    const first =
      body.querySelector<HTMLElement>(`[data-card="${CSS.escape(id)}"] .accounts-menu-item[aria-checked="true"]`) ??
      body.querySelector<HTMLElement>(`[data-card="${CSS.escape(id)}"] .accounts-menu-item, [data-card="${CSS.escape(id)}"] [data-action="email-menu-retry"]`);
    first?.focus();
  }

  function closeEmailMenu(focusTrigger: boolean): void {
    const m = state.emailMenu;
    if (!m) return;
    state.emailMenu = null;
    updateCard(m.accountId);
    if (focusTrigger) focusIn(m.accountId, '[data-action="email-menu"]');
  }

  async function pickEmail(id: string, email: string): Promise<void> {
    const m = state.emailMenu;
    if (!m || m.accountId !== id || m.saving) return;
    if (m.data?.emails.find((o) => o.email === email)?.selected) {
      closeEmailMenu(true);
      return;
    }
    m.saving = email;
    updateCard(id);
    try {
      const res = await accountsApi.setEmail(id, email);
      if (disposed) return;
      replaceAccount(res.account);
      if (state.emailMenu === m) state.emailMenu = null;
      updateCard(id);
      focusIn(id, '[data-action="email-menu"]');
      const repos = res.repos_updated === 1 ? "1 repositório atualizado" : `${res.repos_updated} repositórios atualizados`;
      showToast(res.repos_updated ? `Email de commit alterado (${repos}).` : "Email de commit alterado.", "ok");
      if (res.repos_failed.length) {
        showToast(`Não foi possível atualizar o email em ${res.repos_failed.length === 1 ? "1 repositório" : `${res.repos_failed.length} repositórios`}:\n${res.repos_failed.join("\n")}`);
      }
      ctx.notifyDataChanged();
    } catch (e) {
      if (disposed) return;
      m.saving = null;
      m.error = errMessage(e);
      updateCard(id);
    }
  }

  async function refreshProfile(btn: HTMLButtonElement, id: string): Promise<void> {
    try {
      const row = await withBusy(btn, "", () => accountsApi.refreshProfile(id));
      if (disposed) return;
      replaceAccount(row);
      state.checks.delete(id);
      void runCheck(id);
      showToast(`Perfil de ${handle(row)} atualizado.`, "ok");
      ctx.notifyDataChanged();
    } catch (e) {
      showToast(errMessage(e));
      if (!disposed) void runCheck(id);
    }
  }

  function confirmRemove(opener: HTMLElement, a: AccountInfo): void {
    if (modal) return;
    modal = new RemoveAccountModal(opener, a, async () => {
      const res = await accountsApi.remove(a.id);
      state.checks.delete(a.id);
      const n = res.removed_bindings;
      showToast(
        n === 0 ? `Conta ${handle(a)} removida.` : `Conta ${handle(a)} removida (${n === 1 ? "1 vínculo removido" : `${n} vínculos removidos`}).`,
        "ok"
      );
      if (!disposed) await load();
      ctx.notifyDataChanged();
    }, () => {
      modal = null;
      if (!disposed && !opener.isConnected) (headerConnect.hidden ? root : headerConnect).focus();
    });
  }

  function focusIn(id: string, sel: string): void {
    body.querySelector<HTMLElement>(`[data-card="${CSS.escape(id)}"] ${sel}`)?.focus();
  }

  // ---------- Eventos ----------

  root.addEventListener("click", (ev) => {
    const target = (ev.target as HTMLElement).closest<HTMLElement>("[data-action]");
    if (state.emailMenu && !(ev.target as HTMLElement).closest(".accounts-email-menu, [data-action='email-menu']")) {
      closeEmailMenu(false);
    }
    if (!target || (target as HTMLButtonElement).disabled) return;
    const id = target.dataset.accountId;
    switch (target.dataset.action) {
      case "connect":
        openFlow(target);
        break;
      case "reconnect":
        openFlow(target, findAccount(id));
        break;
      case "repos":
        ctx.navigate("repos");
        break;
      case "reload":
        void load();
        break;
      case "edit-label":
        if (id) startEdit(id);
        break;
      case "email-menu":
        if (!id) break;
        if (state.emailMenu?.accountId === id) closeEmailMenu(true);
        else void openEmailMenu(id);
        break;
      case "email-menu-retry":
        if (id) void openEmailMenu(id);
        break;
      case "email-menu-close":
        closeEmailMenu(true);
        break;
      case "email-pick":
        if (id && target.dataset.email) void pickEmail(id, target.dataset.email);
        break;
      case "refresh":
        if (id) void refreshProfile(target as HTMLButtonElement, id);
        break;
      case "recheck":
        if (id) void runCheck(id);
        break;
      case "remove": {
        const a = findAccount(id);
        if (a) confirmRemove(target, a);
        break;
      }
    }
  });

  root.addEventListener("input", (ev) => {
    const el = ev.target as HTMLElement;
    if (el instanceof HTMLInputElement && el.dataset.action === "label-input" && state.editing) {
      state.editing.draft = el.value;
    }
  });

  root.addEventListener("keydown", (ev) => {
    const el = ev.target as HTMLElement;
    if (el.dataset.action === "label-input") {
      if (ev.key === "Enter") {
        ev.preventDefault();
        void saveEdit();
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        cancelEdit();
      }
      return;
    }
    if (state.emailMenu && el.closest(".accounts-email-menu")) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        closeEmailMenu(true);
      } else if (ev.key === "ArrowDown" || ev.key === "ArrowUp" || ev.key === "Home" || ev.key === "End") {
        const items = [...el.closest(".accounts-email-menu")!.querySelectorAll<HTMLElement>(".accounts-menu-item:not([disabled])")];
        if (!items.length) return;
        ev.preventDefault();
        const i = items.indexOf(el);
        const next =
          ev.key === "Home" ? 0 : ev.key === "End" ? items.length - 1 : ev.key === "ArrowDown" ? (i + 1) % items.length : (i - 1 + items.length) % items.length;
        items[next].focus();
      }
    }
  });

  root.addEventListener(
    "focusout",
    (ev) => {
      const el = ev.target as HTMLElement;
      if (rendering || el.dataset.action !== "label-input" || !state.editing || state.editing.saving) return;
      void saveEdit(!(ev as FocusEvent).relatedTarget);
    },
    true
  );

  /* `error` não borbulha: escuta na fase de captura para trocar o avatar do GitHub pelas iniciais. */
  root.addEventListener(
    "error",
    (ev) => {
      const img = ev.target;
      if (img instanceof HTMLImageElement && img.classList.contains("accounts-avatar-img")) {
        failedAvatars.add(img.getAttribute("src") ?? "");
        img.remove();
      }
    },
    true
  );

  const onDataChanged = () => void load();
  document.addEventListener(DATA_CHANGED_EVENT, onDataChanged);
  void load();

  return () => {
    disposed = true;
    document.removeEventListener(DATA_CHANGED_EVENT, onDataChanged);
    modal?.close();
    modal = null;
  };
};

function displayName(a: AccountInfo): string {
  return a.label || a.name || a.login || a.id.slice(0, 8);
}

function handle(a: Pick<AccountInfo, "login" | "label" | "id">): string {
  return a.login ? `@${a.login}` : a.label || a.id.slice(0, 8);
}

function isNoreply(email: string): boolean {
  return /@users\.noreply\.github\.com$/i.test(email);
}

function scopeLabel(scope: string): string {
  return SCOPES.find((s) => s.scope === scope)?.label.toLowerCase() ?? scope;
}

function sizedAvatar(url: string, px: number): string {
  try {
    const u = new URL(url);
    if (!/^https:$/.test(u.protocol)) return "";
    u.searchParams.set("s", String(px));
    return u.toString();
  } catch {
    return "";
  }
}

function renderSkeleton(): string {
  const card = `
    <div class="panel accounts-card accounts-card-skeleton" aria-hidden="true">
      <div class="accounts-card-head">
        <div class="skeleton accounts-sk-avatar"></div>
        <div class="accounts-card-id"><div class="skeleton accounts-sk-line"></div><div class="skeleton accounts-sk-line short"></div></div>
      </div>
      <div class="skeleton accounts-sk-line"></div>
      <div class="skeleton accounts-sk-line short"></div>
    </div>`;
  return `<p class="sr-only">Carregando contas…</p><div class="accounts-grid">${card}${card}</div>`;
}

class RemoveAccountModal extends ModalBase {
  private busy = false;

  constructor(
    opener: HTMLElement,
    private account: AccountInfo,
    private onConfirm: () => Promise<void>,
    onClosed: () => void
  ) {
    super(opener, "Remover conta?", "accounts-remove-modal", onClosed, "alertdialog");
    this.render();
  }

  private render(error?: string): void {
    const a = this.account;
    const n = a.repos_count;
    const bindings =
      n === 0
        ? "Esta conta não tem repositórios vinculados."
        : n === 1
          ? "O vínculo de <strong>1 repositório</strong> a esta conta é removido."
          : `Os vínculos de <strong>${n} repositórios</strong> a esta conta são removidos.`;
    this.setContent(
      `
      <div class="accounts-remove-who">
        ${profileAvatar({ id: a.id, login: a.login, label: a.label || a.name }, 36)}
        <div class="accounts-card-id">
          <strong class="truncate">${esc(displayName(a))}</strong>
          ${a.login ? `<span class="muted small mono truncate">@${esc(a.login)}</span>` : ""}
        </div>
      </div>
      <ul class="accounts-remove-list small" id="accounts-modal-desc">
        <li>${bindings}</li>
        <li>O acesso desta conta é apagado das Chaves do macOS (Keychain).</li>
        <li>As pastas e o histórico dos repositórios não são excluídos.</li>
      </ul>
      <p class="small muted accounts-remove-note">Para retirar também a autorização do lado do GitHub, abra os <a href="${GITHUB_APPS_URL}">apps autorizados no GitHub</a>.</p>
      ${
        error
          ? `<div class="callout error" role="alert">${icon("alert")}<div class="accounts-callout-body"><strong>Não foi possível remover a conta.</strong><span>${esc(error)}</span></div></div>`
          : ""
      }
      <div class="accounts-modal-foot">
        <button type="button" class="btn ghost" data-modal="close">Cancelar</button>
        <button type="button" class="btn accounts-btn-danger" data-modal="confirm">${trashIcon()}<span>Remover conta</span></button>
      </div>`,
      '[data-modal="close"].btn:not(.icon-only)'
    );
  }

  protected canClose(): boolean {
    return !this.busy;
  }

  protected onAction(action: string, el: HTMLElement): void {
    if (action !== "confirm" || this.busy) return;
    this.busy = true;
    void withBusy(el as HTMLButtonElement, "Removendo…", this.onConfirm)
      .then(() => {
        this.busy = false;
        this.close();
      })
      .catch((e) => {
        this.busy = false;
        if (!this.closed) this.render(errMessage(e));
      });
  }
}

function focusKey(el: HTMLElement): string | null {
  const action = el.dataset.action;
  if (!action) return null;
  return `${action}|${el.dataset.accountId ?? ""}|${el.dataset.email ?? ""}`;
}
