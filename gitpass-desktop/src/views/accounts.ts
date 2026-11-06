import "./accounts.css";
import { api, type AccountRow, type BindingRow, type DeviceCodeStart, type OauthConfigStatus } from "../lib/api";
import { esc, showToast } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileAvatar, profileFamily, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView, type ViewContext } from "../lib/view";

type State = {
  loading: boolean;
  error: string | null;
  accounts: AccountRow[];
  bindings: BindingRow[];
  oauth: OauthConfigStatus | null;
  oauthError: string | null;
};

const SUCCESS_CLOSE_MS = 1500;
const COPIED_RESET_MS = 2000;

export const mount: MountView = (root, ctx) => {
  const state: State = { loading: true, error: null, accounts: [], bindings: [], oauth: null, oauthError: null };
  let loadSeq = 0;
  let disposed = false;
  let modal: DeviceFlowModal | null = null;

  root.innerHTML = `
    <header class="view-header">
      <div>
        <h1 class="view-title">Contas</h1>
        <p class="view-sub accounts-sub" data-ref="sub">A carregar…</p>
      </div>
      <div class="view-actions">
        <button type="button" class="btn primary" data-action="connect">
          ${icon("plus")}<span>Ligar conta GitHub</span>
        </button>
      </div>
    </header>
    <div class="accounts-body" data-ref="body" aria-live="polite" aria-busy="true"></div>
  `;

  const sub = root.querySelector<HTMLElement>('[data-ref="sub"]')!;
  const body = root.querySelector<HTMLElement>('[data-ref="body"]')!;
  const headerConnect = root.querySelector<HTMLButtonElement>('[data-action="connect"]')!;

  const oauthReady = () => state.oauth?.ok === true;

  function openFlow(opener: HTMLElement, account?: AccountRow): void {
    if (modal || !oauthReady()) return;
    modal = new DeviceFlowModal(ctx, opener, account ?? null, () => {
      modal = null;
    });
  }

  root.addEventListener("click", (ev) => {
    const target = (ev.target as HTMLElement).closest<HTMLElement>("[data-action]");
    if (!target || (target as HTMLButtonElement).disabled) return;
    const action = target.dataset.action;
    if (action === "connect") openFlow(target);
    else if (action === "reconnect") {
      const acc = state.accounts.find((a) => a.id === target.dataset.accountId);
      openFlow(target, acc);
    } else if (action === "repos") ctx.navigate("repos");
    else if (action === "reload") void load();
  });

  async function load(): Promise<void> {
    const seq = ++loadSeq;
    state.loading = true;
    state.error = null;
    if (!state.accounts.length) render();
    const [oauthRes, accRes, bindRes] = await Promise.allSettled([api.oauthStatus(), api.accounts(), api.bindings()]);
    if (disposed || seq !== loadSeq) return;
    state.loading = false;
    if (oauthRes.status === "fulfilled") {
      state.oauth = oauthRes.value;
      state.oauthError = null;
    } else {
      state.oauth = null;
      state.oauthError = String(oauthRes.reason);
    }
    if (accRes.status === "fulfilled") state.accounts = accRes.value;
    else state.error = String(accRes.reason);
    state.bindings = bindRes.status === "fulfilled" ? bindRes.value : [];
    render();
  }

  function render(): void {
    body.setAttribute("aria-busy", String(state.loading));
    const n = state.accounts.length;
    const ready = oauthReady();

    headerConnect.disabled = state.loading || !ready;
    headerConnect.title = ready ? "" : "Indisponível: a configuração OAuth não está pronta.";
    if (ready || state.loading) headerConnect.removeAttribute("aria-describedby");
    else headerConnect.setAttribute("aria-describedby", "accounts-oauth-hint");

    if (state.loading && !n) {
      sub.textContent = "A carregar…";
      body.innerHTML = renderSkeleton();
      return;
    }

    const countText = n === 0 ? "Nenhuma conta ligada" : n === 1 ? "1 conta ligada" : `${n} contas ligadas`;
    sub.innerHTML = `<span>${esc(countText)}</span>${
      ready
        ? `<span class="badge success accounts-oauth-badge" title="GitHub Device Flow configurado">${icon("check", 12)}OAuth pronto</span>`
        : ""
    }`;

    let html = renderOauthBanner();
    if (state.error) {
      html += `
        <div class="callout error accounts-error" role="alert">
          ${icon("alert")}
          <div class="accounts-callout-body">
            <strong>Não foi possível carregar as contas.</strong>
            <span>${esc(state.error)}</span>
            <div><button type="button" class="btn" data-action="reload">${icon("refresh")}<span>Tentar novamente</span></button></div>
          </div>
        </div>`;
    } else if (n === 0) {
      html += renderEmpty(ready);
    } else {
      html += `<div class="accounts-grid">${state.accounts.map((a) => renderCard(a, ready)).join("")}</div>`;
    }
    body.innerHTML = html;
  }

  function renderOauthBanner(): string {
    if (state.loading || oauthReady()) return "";
    const message = state.oauthError ?? state.oauth?.message ?? "Configuração OAuth indisponível.";
    const path = state.oauth?.oauth_env_path;
    return `
      <div class="callout error accounts-oauth-callout" role="alert" id="accounts-oauth-hint">
        ${icon("alert")}
        <div class="accounts-callout-body">
          <strong>Não é possível ligar contas GitHub de momento.</strong>
          <span class="accounts-pre">${esc(message)}</span>
          ${path ? `<span>Ficheiro de configuração: <span class="mono">${esc(path)}</span></span>` : ""}
          <span class="accounts-callout-note">O GitPass usa o GitHub Device Flow: basta um <span class="mono">client_id</span> público, sem <span class="mono">client_secret</span>. O botão «Ligar conta GitHub» fica indisponível até a configuração estar correta.</span>
        </div>
      </div>`;
  }

  function renderEmpty(ready: boolean): string {
    return `
      <section class="panel accounts-empty">
        <div class="empty-state">
          <div class="empty-icon">${icon("user", 28)}</div>
          <h3>Nenhuma conta ligada</h3>
          <p>Ligue uma conta GitHub para associar repositórios a um perfil. O GitPass mostra-lhe um código curto; abre o GitHub no browser, introduz o código e autoriza. O token fica guardado no Keychain.</p>
          <button type="button" class="btn primary lg" data-action="connect" ${ready ? "" : 'disabled aria-describedby="accounts-oauth-hint"'}>
            ${icon("plus")}<span>Ligar conta GitHub</span>
          </button>
        </div>
      </section>`;
  }

  function renderCard(a: AccountRow, ready: boolean): string {
    const name = profileName(a);
    const fam = profileFamily(a.id);
    const repoCount = state.bindings.filter((b) => b.account_id === a.id).length;
    const titleId = `accounts-card-${esc(a.id)}`;
    const repoText = repoCount === 0 ? "Sem repositórios locais" : repoCount === 1 ? "1 repositório local" : `${repoCount} repositórios locais`;
    const tokenBadge = a.has_token
      ? `<span class="badge success">${icon("key", 12)}Token no Keychain</span>`
      : `<span class="badge warning">${icon("alert", 12)}Sem token</span>`;

    return `
      <article class="panel accounts-card profile-${fam}" aria-labelledby="${titleId}">
        <div class="accounts-card-head">
          ${profileAvatar(a, 48)}
          <div class="accounts-card-id">
            <h3 class="panel-title truncate" id="${titleId}">${esc(name)}</h3>
            <p class="mono muted small truncate">${a.login ? `@${esc(a.login)}` : "sem login"}</p>
          </div>
        </div>
        <div class="accounts-card-badges">
          <span class="badge" title="Tipo de autenticação">${icon("lock", 12)}${esc(authLabel(a.auth))}</span>
          ${tokenBadge}
        </div>
        ${
          a.has_token
            ? ""
            : `<div class="accounts-card-warn">
                <span class="small">O token não está no Keychain. Volte a ligar para continuar a usar este perfil.</span>
                <button type="button" class="btn" data-action="reconnect" data-account-id="${esc(a.id)}" ${ready ? "" : "disabled"}>
                  ${icon("refresh")}<span>Voltar a ligar</span>
                </button>
              </div>`
        }
        <div class="accounts-card-foot">
          <span class="accounts-repo-count small">${icon("folder", 14)}<span>${esc(repoText)}</span></span>
          <button type="button" class="btn ghost" data-action="repos">${icon("repo")}<span>Ver repositórios</span></button>
        </div>
      </article>`;
  }

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

function authLabel(auth: string): string {
  const a = auth.toLowerCase();
  if (a.includes("device") || a.includes("oauth")) return "OAuth";
  if (a.includes("pat") || a.includes("token")) return "Token pessoal";
  if (a.includes("ssh")) return "SSH";
  return auth || "Desconhecido";
}

function renderSkeleton(): string {
  const card = `
    <div class="panel accounts-card accounts-card-skeleton" aria-hidden="true">
      <div class="accounts-card-head">
        <div class="skeleton accounts-sk-avatar"></div>
        <div class="accounts-card-id"><div class="skeleton accounts-sk-line"></div><div class="skeleton accounts-sk-line short"></div></div>
      </div>
      <div class="skeleton accounts-sk-line short"></div>
    </div>`;
  return `<p class="sr-only">A carregar contas…</p><div class="accounts-grid">${card}${card}</div>`;
}

function formatClock(totalSecs: number): string {
  const s = Math.max(0, Math.ceil(totalSecs));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

/** Token de cancelamento: cada tentativa do fluxo tem o seu; cancelar acorda esperas pendentes. */
class FlowToken {
  cancelled = false;
  private handlers: (() => void)[] = [];

  cancel(): void {
    if (this.cancelled) return;
    this.cancelled = true;
    for (const h of this.handlers.splice(0)) h();
  }

  onCancel(fn: () => void): void {
    if (this.cancelled) fn();
    else this.handlers.push(fn);
  }

  sleep(ms: number): Promise<void> {
    return new Promise((resolve) => {
      const t = window.setTimeout(resolve, ms);
      this.onCancel(() => {
        window.clearTimeout(t);
        resolve();
      });
    });
  }
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])';

class DeviceFlowModal {
  private backdrop: HTMLDivElement;
  private dialog: HTMLElement;
  private content: HTMLElement;
  private token: FlowToken | null = null;
  private closed = false;
  private closeTimer: number | undefined;

  constructor(
    private ctx: ViewContext,
    private opener: HTMLElement,
    private account: AccountRow | null,
    private onClosed: () => void
  ) {
    this.backdrop = document.createElement("div");
    this.backdrop.className = "modal-backdrop accounts-modal-backdrop";
    this.backdrop.innerHTML = `
      <div class="modal accounts-device-modal" role="dialog" aria-modal="true" aria-labelledby="accounts-device-title" aria-describedby="accounts-device-desc">
        <header class="accounts-modal-head">
          <h2 class="accounts-modal-title" id="accounts-device-title">${account ? "Voltar a ligar conta" : "Ligar conta GitHub"}</h2>
          <button type="button" class="btn ghost icon-only" data-modal="close" aria-label="Cancelar e fechar">${icon("x")}</button>
        </header>
        <div class="accounts-modal-content" data-ref="content"></div>
      </div>`;
    this.dialog = this.backdrop.querySelector<HTMLElement>(".modal")!;
    this.content = this.backdrop.querySelector<HTMLElement>('[data-ref="content"]')!;

    this.backdrop.addEventListener("keydown", this.onKeyDown);
    this.backdrop.addEventListener("click", this.onClick);
    document.body.appendChild(this.backdrop);
    void this.start();
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.token?.cancel();
    window.clearTimeout(this.closeTimer);
    this.backdrop.removeEventListener("keydown", this.onKeyDown);
    this.backdrop.removeEventListener("click", this.onClick);
    this.backdrop.remove();
    if (this.opener.isConnected) this.opener.focus();
    this.onClosed();
  }

  private onKeyDown = (ev: KeyboardEvent): void => {
    if (ev.key === "Escape") {
      ev.preventDefault();
      this.close();
      return;
    }
    if (ev.key !== "Tab") return;
    const items = [...this.dialog.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null);
    if (!items.length) {
      ev.preventDefault();
      this.dialog.focus();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (ev.shiftKey && (active === first || !this.dialog.contains(active))) {
      ev.preventDefault();
      last.focus();
    } else if (!ev.shiftKey && (active === last || !this.dialog.contains(active))) {
      ev.preventDefault();
      first.focus();
    }
  };

  private onClick = (ev: MouseEvent): void => {
    const el = (ev.target as HTMLElement).closest<HTMLElement>("[data-modal]");
    if (!el) return;
    const action = el.dataset.modal;
    if (action === "close") this.close();
    else if (action === "retry") void this.start();
    else if (action === "copy") void this.copy(el as HTMLButtonElement);
  };

  private setContent(html: string, focusSel?: string): void {
    this.content.innerHTML = html;
    const target = (focusSel && this.dialog.querySelector<HTMLElement>(focusSel)) || this.dialog;
    if (target === this.dialog) this.dialog.setAttribute("tabindex", "-1");
    target.focus();
  }

  private async start(): Promise<void> {
    this.token?.cancel();
    const token = new FlowToken();
    this.token = token;

    this.setContent(`
      <div class="accounts-step accounts-step-loading" role="status">
        <span class="spinner accounts-spinner-lg" aria-hidden="true"></span>
        <p id="accounts-device-desc">A pedir código ao GitHub…</p>
      </div>`);

    let start: DeviceCodeStart;
    try {
      start = await api.deviceStart();
    } catch (e) {
      if (token.cancelled) return;
      this.showError("Não foi possível obter um código do GitHub.", String(e));
      return;
    }
    if (token.cancelled) return;

    const deadline = Date.now() + Math.max(1, start.expires_in_secs) * 1000;
    this.showCode(start, deadline, token);
    void this.poll(start, deadline, token);
  }

  private showCode(start: DeviceCodeStart, deadline: number, token: FlowToken): void {
    const who = this.account?.login
      ? `Inicie sessão no GitHub como <span class="mono">@${esc(this.account.login)}</span> e introduza este código:`
      : "Introduza este código no GitHub para autorizar o GitPass:";

    this.setContent(
      `
      <p class="accounts-modal-lead" id="accounts-device-desc">${who}</p>
      <div class="accounts-code-box">
        <output class="accounts-user-code mono" aria-label="Código de verificação ${esc(start.user_code.split("").join(" "))}">${esc(start.user_code)}</output>
        <button type="button" class="btn" data-modal="copy" data-code="${esc(start.user_code)}">
          <span class="accounts-copy-icon">${icon("copy")}</span><span class="accounts-copy-label">Copiar código</span>
        </button>
      </div>
      <ol class="accounts-steps small">
        <li>Copie o código acima.</li>
        <li>Abra o GitHub no browser e cole o código.</li>
        <li>Autorize o GitPass — esta janela fecha sozinha.</li>
      </ol>
      <div class="accounts-open">
        <a class="btn primary lg" href="${esc(start.verification_uri)}" target="_blank" rel="noopener noreferrer">
          ${icon("external")}<span>Abrir GitHub</span>
        </a>
        <span class="mono small muted accounts-uri">${esc(start.verification_uri)}</span>
      </div>
      <div class="accounts-wait">
        <span class="accounts-wait-status" role="status"><span class="spinner" aria-hidden="true"></span>À espera da autorização no browser…</span>
        <span class="accounts-countdown small muted" data-ref="countdown">Expira em ${formatClock((deadline - Date.now()) / 1000)}</span>
      </div>
      <div class="accounts-modal-foot">
        <button type="button" class="btn ghost" data-modal="close">Cancelar</button>
      </div>`,
      '[data-modal="copy"]'
    );

    const countdown = this.content.querySelector<HTMLElement>('[data-ref="countdown"]')!;
    const tick = window.setInterval(() => {
      const left = (deadline - Date.now()) / 1000;
      if (left <= 0) {
        window.clearInterval(tick);
        if (!token.cancelled) this.showExpired(token);
        return;
      }
      countdown.textContent = `Expira em ${formatClock(left)}`;
      countdown.classList.toggle("accounts-countdown-low", left <= 60);
    }, 1000);
    token.onCancel(() => window.clearInterval(tick));
  }

  private async poll(start: DeviceCodeStart, deadline: number, token: FlowToken): Promise<void> {
    let intervalSecs = Math.max(1, start.interval_secs);
    while (!token.cancelled) {
      await token.sleep(intervalSecs * 1000);
      if (token.cancelled) return;
      if (Date.now() >= deadline) {
        this.showExpired(token);
        return;
      }
      try {
        const res = await api.devicePoll(start.client_id, start.device_code);
        if (token.cancelled) return;
        if (res.status === "slow_down") {
          intervalSecs = Math.max(1, intervalSecs, res.interval_secs);
        } else if (res.status === "done") {
          this.showSuccess(res.message, token);
          return;
        }
      } catch (e) {
        if (token.cancelled) return;
        this.showError("A autorização falhou.", String(e));
        return;
      }
    }
  }

  private showSuccess(message: string, token: FlowToken): void {
    token.cancel();
    this.setContent(
      `
      <div class="accounts-step accounts-step-success" role="status">
        <span class="accounts-result-icon success">${icon("check", 28)}</span>
        <p class="accounts-result-title" id="accounts-device-desc">Conta ligada</p>
        <p class="small muted accounts-pre">${esc(message)}</p>
      </div>`,
      '[data-modal="close"]'
    );
    showToast(message || "Conta GitHub ligada.", "ok");
    this.ctx.notifyDataChanged();
    this.closeTimer = window.setTimeout(() => this.close(), SUCCESS_CLOSE_MS);
  }

  private showExpired(token: FlowToken): void {
    token.cancel();
    this.setContent(
      `
      <div class="callout warning" role="alert">
        ${icon("alert")}
        <div class="accounts-callout-body">
          <strong id="accounts-device-desc">O código expirou.</strong>
          <span>A autorização não foi concluída a tempo. Peça um novo código para tentar outra vez.</span>
        </div>
      </div>
      <div class="accounts-modal-foot">
        <button type="button" class="btn ghost" data-modal="close">Cancelar</button>
        <button type="button" class="btn primary" data-modal="retry">${icon("refresh")}<span>Pedir novo código</span></button>
      </div>`,
      '[data-modal="retry"]'
    );
  }

  private showError(title: string, detail: string): void {
    this.token?.cancel();
    this.setContent(
      `
      <div class="callout error" role="alert">
        ${icon("alert")}
        <div class="accounts-callout-body">
          <strong id="accounts-device-desc">${esc(title)}</strong>
          <span class="accounts-pre">${esc(detail)}</span>
        </div>
      </div>
      <div class="accounts-modal-foot">
        <button type="button" class="btn ghost" data-modal="close">Cancelar</button>
        <button type="button" class="btn primary" data-modal="retry">${icon("refresh")}<span>Tentar novamente</span></button>
      </div>`,
      '[data-modal="retry"]'
    );
  }

  private async copy(btn: HTMLButtonElement): Promise<void> {
    const code = btn.dataset.code ?? "";
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      showToast("Não foi possível copiar. Selecione o código e copie manualmente.");
      return;
    }
    const iconEl = btn.querySelector<HTMLElement>(".accounts-copy-icon");
    const label = btn.querySelector<HTMLElement>(".accounts-copy-label");
    if (!iconEl || !label) return;
    iconEl.innerHTML = icon("check");
    label.textContent = "Copiado";
    btn.classList.add("accounts-copied");
    window.setTimeout(() => {
      if (!btn.isConnected) return;
      iconEl.innerHTML = icon("copy");
      label.textContent = "Copiar código";
      btn.classList.remove("accounts-copied");
    }, COPIED_RESET_MS);
  }
}
