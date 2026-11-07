import "./accounts.css";
import { api, type OauthConfigStatus } from "../lib/api";
import { accountsApi, type AccountCheck, type AccountEmails, type AccountInfo } from "../lib/api-accounts";
import { errMessage, esc, showToast, withBusy } from "../lib/dom";
import { errorView } from "../lib/errors";
import { DeviceFlowModal, ModalBase } from "../lib/device-flow";
import { icon } from "../lib/icons";
import { profileAvatar, profileFamily } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView, type ViewContext } from "../lib/view";
import { openMenu, type MenuItem } from "./repos-ui";

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
    <div class="page-head">
      <div class="heading-group">
        <h1>Contas</h1>
        <p data-ref="sub">Gerencie os perfis usados nos seus projetos.</p>
      </div>
      <button type="button" class="control primary" data-action="connect" hidden>${icon("plus")}<span>Conectar conta</span></button>
    </div>
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
      body.innerHTML = renderSkeleton();
      return;
    }

    let html = ready ? "" : renderOauthUnavailable();
    if (state.error) {
      html += errorView({
        title: "Não foi possível carregar as contas",
        error: state.error,
        actions: [{ label: "Tentar de novo", action: "reload", icon: "refresh", primary: true }],
      });
    } else if (n === 0) {
      html += renderEmpty(ready);
    } else {
      html += `<div class="accounts">${state.accounts.map((a) => renderCard(a)).join("")}</div>`;
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
      <section class="surface accounts-empty">
        <div class="empty-state">
          <div class="empty-icon">${icon("user", 28)}</div>
          <h3>Nenhuma conta conectada</h3>
          ${
            ready
              ? `<p>Conecte uma conta do GitHub para vincular repositórios a um perfil. O GitPass mostra um código curto. Você abre o GitHub no navegador, digita o código e autoriza. O acesso fica salvo nas Chaves do macOS (Keychain).</p>
                 <button type="button" class="control primary" data-action="connect">${icon("plus")}<span>Conectar conta</span></button>`
              : `<p>Quando a conexão com o GitHub estiver disponível, você poderá conectar suas contas aqui.</p>`
          }
        </div>
      </section>`;
  }

  function renderCard(a: AccountInfo): string {
    const fam = profileFamily(a.id);
    const titleId = `accounts-card-${esc(a.id)}`;
    const check = state.checks.get(a.id);
    const repoText = a.repos_count === 0 ? "Nenhum repositório" : a.repos_count === 1 ? "1 repositório" : `${a.repos_count} repositórios`;
    return `
      <section class="surface account-card ${fam}" data-card="${esc(a.id)}" aria-labelledby="${titleId}">
        <div class="account-top">
          ${renderAvatar(a)}
          <div class="account-id">${renderIdentity(a, titleId)}</div>
          <button type="button" class="icon-button account-more" data-action="more" data-account-id="${esc(a.id)}" aria-haspopup="menu" aria-label="Mais ações para ${esc(
            handle(a)
          )}" title="Mais ações">${icon("more")}</button>
        </div>
        <div class="account-email">
          <p class="caption muted">E-mail de commit</p>
          ${renderEmail(a)}
        </div>
        ${state.emailMenu?.accountId === a.id ? renderEmailMenu(a, state.emailMenu) : ""}
        <div class="account-state">${renderTokenState(a, check)}</div>
        ${renderAttention(a, check)}
        <div class="account-foot"><span class="caption muted">${esc(repoText)}</span>
          <button class="control" type="button" data-action="repos" data-account-id="${esc(a.id)}"${a.repos_count ? "" : " disabled"}>Ver projetos${icon("chevronRight")}</button>
        </div>
      </section>`;
  }

  function renderAvatar(a: AccountInfo): string {
    const fallback = profileAvatar({ id: a.id, login: a.login, label: a.label || a.name }, 42);
    const src = a.avatar_url ? sizedAvatar(a.avatar_url, 84) : null;
    const img =
      src && !failedAvatars.has(src)
        ? `<img class="accounts-avatar-img" src="${esc(src)}" alt="" width="42" height="42" decoding="async" referrerpolicy="no-referrer" />`
        : "";
    return `<span class="accounts-avatar">${fallback}${img}</span>`;
  }

  function renderIdentity(a: AccountInfo, titleId: string): string {
    const title = displayName(a);
    const secondary = [a.login ? `@${a.login}` : null, a.label && a.name && a.name !== a.label ? a.name : null].filter(Boolean).join(" · ");
    const ed = state.editing;
    if (ed?.id === a.id) {
      return `
        <label class="sr-only" for="accounts-label-${esc(a.id)}">Nome do perfil ${esc(handle(a))}</label>
        <input id="accounts-label-${esc(a.id)}" class="input accounts-label-input" data-action="label-input" data-account-id="${esc(a.id)}"
          value="${esc(ed.draft)}" maxlength="${LABEL_MAX}" placeholder="${esc(a.name || a.login || "Nome do perfil")}"
          autocomplete="off" spellcheck="false" aria-describedby="accounts-label-hint-${esc(a.id)}" ${ed.saving ? "disabled" : ""} />
        <p class="caption muted accounts-label-hint" id="accounts-label-hint-${esc(a.id)}">${
          ed.saving ? "Salvando…" : "Enter salva · Esc cancela · vazio volta ao nome do GitHub"
        }</p>
        <h2 class="sr-only" id="${titleId}">${esc(title)}</h2>`;
    }
    return `<h2 class="truncate" id="${titleId}">${esc(title)}</h2><p class="muted truncate">${secondary ? esc(secondary) : "sem login"}</p>`;
  }

  function renderEmail(a: AccountInfo): string {
    const open = state.emailMenu?.accountId === a.id;
    const value = a.email
      ? `<code class="truncate" title="${esc(a.email)}">${esc(a.email)}</code>${
          isNoreply(a.email) ? `<span class="account-tag" title="Endereço privado do GitHub: seu e-mail pessoal não aparece nos commits">${icon("lock", 12)}Privado</span>` : ""
        }`
      : `<code class="muted">Não definido</code>`;
    return `<div class="account-email-value">${value}${
      a.has_token
        ? `<button type="button" class="account-email-btn" data-action="email-menu" data-account-id="${esc(a.id)}" aria-haspopup="menu" aria-expanded="${open}" aria-controls="accounts-email-menu-${esc(
            a.id
          )}" aria-label="Alterar o e-mail de commit de ${esc(handle(a))}" title="Alterar o e-mail de commit">${icon("edit", 13)}</button>`
        : ""
    }</div>`;
  }

  function renderEmailMenu(a: AccountInfo, m: EmailMenu): string {
    let inner: string;
    if (m.loading) {
      inner = `<p class="accounts-menu-status caption muted" role="status"><span class="spinner" aria-hidden="true"></span>Buscando e-mails verificados…</p>`;
    } else if (m.error) {
      inner = `
        <p class="accounts-menu-status caption err" role="alert">${icon("alert")}<span>${esc(m.error)}</span></p>
        <div><button type="button" class="control" data-action="email-menu-retry" data-account-id="${esc(a.id)}">${icon("refresh")}<span>Tentar de novo</span></button></div>`;
    } else {
      const opts = m.data?.emails ?? [];
      inner = `
        <div class="accounts-menu-list" role="menu" aria-label="E-mails verificados de ${esc(handle(a))}">
          ${opts
            .map((o) => {
              const tag = o.primary ? "Principal" : o.noreply ? "Privado" : "";
              const busy = m.saving === o.email;
              return `
                <button type="button" class="accounts-menu-item" role="menuitemradio" aria-checked="${o.selected}"
                  data-action="email-pick" data-account-id="${esc(a.id)}" data-email="${esc(o.email)}" ${m.saving ? "disabled" : ""}>
                  <span class="accounts-menu-check" aria-hidden="true">${busy ? '<span class="spinner"></span>' : o.selected ? icon("check", 14) : ""}</span>
                  <span class="mono truncate">${esc(o.email)}</span>
                  ${tag ? `<span class="account-tag">${esc(tag)}</span>` : ""}
                </button>`;
            })
            .join("")}
        </div>
        ${
          m.data?.needs_reconnect
            ? `<p class="caption warn accounts-menu-note">${icon("alert")}<span>Sem permissão de e-mail: só aparecem o e-mail público e o privado. Conecte a conta de novo para ver todos os e-mails verificados.</span></p>`
            : ""
        }
        <p class="caption muted accounts-menu-note">Os repositórios vinculados a esta conta passam a usar o e-mail escolhido nos próximos commits.</p>`;
    }
    return `
      <div class="accounts-email-menu" id="accounts-email-menu-${esc(a.id)}">
        <div class="accounts-email-menu-head">
          <span class="caption">Escolher o e-mail de commit</span>
          <button type="button" class="icon-button" data-action="email-menu-close" data-account-id="${esc(a.id)}" aria-label="Fechar a lista de e-mails">${icon("x")}</button>
        </div>
        ${inner}
      </div>`;
  }

  function scopesText(a: AccountInfo, check: CheckState | undefined): string {
    const live = check && check.state !== "checking" && check.scopes.length ? check.scopes : null;
    const scopes = live ?? a.scopes;
    const granted = SCOPES.filter((s) => scopes.includes(s.scope) || s.impliedBy.some((i) => scopes.includes(i))).map((s) => s.label);
    return granted.length ? `Permissões: ${granted.join(", ")}` : "";
  }

  function renderTokenState(a: AccountInfo, check: CheckState | undefined): string {
    const reconnect = oauthReady()
      ? `<button type="button" class="control ghost" data-action="reconnect" data-account-id="${esc(a.id)}">${icon("refresh")}<span>Conectar de novo</span></button>`
      : "";
    if (!a.has_token) return `<span class="state warning">${icon("alert")}Desconectada</span>${reconnect}`;
    if (!check || check.state === "checking") {
      return `<span class="state neutral"><span class="spinner" aria-hidden="true"></span>Verificando o acesso…</span>`;
    }
    const title = `title="${esc([check.message, scopesText(a, check)].filter(Boolean).join(" · "))}"`;
    switch (check.state) {
      case "ok":
        return `<span class="state" ${title}>${icon("check")}Conta conectada</span>`;
      case "revoked":
        return `<span class="state error" ${title}>${icon("x")}Desconectada: acesso revogado</span>${reconnect}`;
      case "missing":
        return `<span class="state warning" ${title}>${icon("alert")}Desconectada</span>${reconnect}`;
      case "offline":
        return `<span class="state neutral" ${title}>${icon("cloud")}Sem conexão</span>
          <button type="button" class="control ghost" data-action="recheck" data-account-id="${esc(a.id)}">Verificar</button>`;
      default:
        return `<span class="state warning" ${title}>${icon("alert")}Não verificado</span>
          <button type="button" class="control ghost" data-action="recheck" data-account-id="${esc(a.id)}">Verificar</button>`;
    }
  }

  function renderAttention(a: AccountInfo, check: CheckState | undefined): string {
    if (!a.has_token || !check || check.state === "checking" || !check.needs_reconnect) return "";
    if (check.state === "revoked" || check.state === "missing") return "";
    const message =
      check.state === "ok" && check.missing_scopes.length
        ? `Faltam permissões (${check.missing_scopes.map(scopeLabel).join(" e ")}). Conecte a conta de novo para dar essas permissões ao GitPass.`
        : check.message;
    return `
      <div class="callout warning accounts-card-warn" role="status">${icon("alert")}
        <span>${esc(message)}</span>
        ${oauthReady() ? `<button type="button" class="control" data-action="reconnect" data-account-id="${esc(a.id)}">${icon("refresh")}<span>Conectar de novo</span></button>` : ""}
      </div>`;
  }

  function openMore(btn: HTMLButtonElement, a: AccountInfo): void {
    const check = state.checks.get(a.id);
    const connected = a.has_token && !(check && (check.state === "revoked" || check.state === "missing"));
    const items: MenuItem[] = [
      { id: "edit-label", html: `${icon("edit")}<span>Renomear perfil</span>` },
      ...(connected ? [{ id: "email-menu", html: `${icon("user")}<span>Alterar o e-mail de commit</span>` }] : []),
      ...(connected ? [{ id: "refresh", html: `${icon("refresh")}<span>Atualizar com os dados do GitHub</span>` }] : []),
      ...(a.has_token ? [{ id: "recheck", html: `${icon("shieldCheck")}<span>Verificar o acesso</span>` }] : []),
      ...(oauthReady() ? [{ id: "reconnect", html: `${icon("key")}<span>Conectar de novo</span>` }] : []),
      { id: "remove", html: `${icon("trash")}<span>Remover conta</span>`, danger: true },
    ];
    openMenu(btn, {
      label: `Ações de ${handle(a)}`,
      items,
      onPick: (id) => {
        switch (id) {
          case "edit-label":
            startEdit(a.id);
            break;
          case "email-menu":
            void openEmailMenu(a.id);
            break;
          case "refresh":
            void refreshProfile(btn, a.id);
            break;
          case "recheck":
            void runCheck(a.id);
            break;
          case "reconnect":
            openFlow(btn, a);
            break;
          case "remove":
            confirmRemove(btn, a);
            break;
        }
      },
    });
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
    if (state.emailMenu && !(ev.target as HTMLElement).closest(".accounts-email-menu, [data-action='email-menu'], .repos-menu")) {
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
        if (id) ctx.navigate(`repos?profile=${encodeURIComponent(id)}`);
        break;
      case "more": {
        const a = findAccount(id);
        if (a) openMore(target as HTMLButtonElement, a);
        break;
      }
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
    <div class="surface account-card accounts-card-skeleton" aria-hidden="true">
      <div class="account-top">
        <div class="skeleton accounts-sk-avatar"></div>
        <div class="account-id"><div class="skeleton accounts-sk-line"></div><div class="skeleton accounts-sk-line short"></div></div>
      </div>
      <div class="skeleton accounts-sk-line"></div>
      <div class="skeleton accounts-sk-line short"></div>
    </div>`;
  return `<p class="sr-only">Carregando contas…</p><div class="accounts">${card}${card}${card}</div>`;
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
        <div class="account-id">
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
        <button type="button" class="control danger" data-modal="confirm">${icon("trash")}<span>Remover conta</span></button>
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
