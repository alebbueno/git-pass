import { type DeviceCodeStart } from "./api";
import { accountsApi, type AccountInfo } from "./api-accounts";
import { errMessage, esc, showToast } from "./dom";
import { icon } from "./icons";
import type { ViewContext } from "./view";
import "../views/accounts.css";

const SUCCESS_CLOSE_MS = 1500;
const COPIED_RESET_MS = 2000;

function formatClock(totalSecs: number): string {
  const s = Math.max(0, Math.ceil(totalSecs));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

/** Token de cancelamento: cada tentativa do fluxo tem o seu; cancelar acorda esperas pendentes. */
export class FlowToken {
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

/** Diálogo modal com focus trap; Escape e o botão `data-modal="close"` fecham. */
export abstract class ModalBase {
  protected backdrop: HTMLDivElement;
  protected dialog: HTMLElement;
  protected content: HTMLElement;
  protected closed = false;

  constructor(
    protected opener: HTMLElement,
    title: string,
    dialogClass: string,
    private onClosed: () => void,
    role: "dialog" | "alertdialog" = "dialog"
  ) {
    this.backdrop = document.createElement("div");
    this.backdrop.className = "modal-backdrop accounts-modal-backdrop";
    this.backdrop.innerHTML = `
      <div class="modal ${dialogClass}" role="${role}" aria-modal="true" aria-labelledby="accounts-modal-title" aria-describedby="accounts-modal-desc">
        <header class="accounts-modal-head">
          <h2 class="accounts-modal-title" id="accounts-modal-title">${esc(title)}</h2>
          <button type="button" class="btn ghost icon-only" data-modal="close" aria-label="Fechar">${icon("x")}</button>
        </header>
        <div class="accounts-modal-content" data-ref="content"></div>
      </div>`;
    this.dialog = this.backdrop.querySelector<HTMLElement>(".modal")!;
    this.content = this.backdrop.querySelector<HTMLElement>('[data-ref="content"]')!;
    this.backdrop.addEventListener("keydown", this.onKeyDown);
    this.backdrop.addEventListener("click", this.onClick);
    document.body.appendChild(this.backdrop);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.onClose();
    this.backdrop.removeEventListener("keydown", this.onKeyDown);
    this.backdrop.removeEventListener("click", this.onClick);
    this.backdrop.remove();
    if (this.opener.isConnected) this.opener.focus();
    this.onClosed();
  }

  protected onClose(): void {}

  protected abstract onAction(action: string, el: HTMLElement): void;

  protected canClose(): boolean {
    return true;
  }

  protected setContent(html: string, focusSel?: string): void {
    this.content.innerHTML = html;
    const target = (focusSel && this.dialog.querySelector<HTMLElement>(focusSel)) || this.dialog;
    if (target === this.dialog) this.dialog.setAttribute("tabindex", "-1");
    target.focus();
  }

  private onKeyDown = (ev: KeyboardEvent): void => {
    if (ev.key === "Escape") {
      ev.preventDefault();
      if (this.canClose()) this.close();
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
    if (!el || (el as HTMLButtonElement).disabled) return;
    if (el.dataset.modal === "close") {
      if (this.canClose()) this.close();
    } else {
      this.onAction(el.dataset.modal ?? "", el);
    }
  };
}

export class DeviceFlowModal extends ModalBase {
  private token: FlowToken | null = null;
  private closeTimer: number | undefined;

  constructor(
    private ctx: ViewContext,
    opener: HTMLElement,
    private account: AccountInfo | null,
    private onDone: (accountId: string) => void,
    onClosed: () => void
  ) {
    super(opener, account ? "Voltar a ligar conta" : "Ligar conta GitHub", "accounts-device-modal", onClosed);
    void this.start();
  }

  protected onClose(): void {
    this.token?.cancel();
    window.clearTimeout(this.closeTimer);
  }

  protected onAction(action: string, el: HTMLElement): void {
    if (action === "retry") void this.start();
    else if (action === "copy") void this.copy(el as HTMLButtonElement);
  }

  private async start(): Promise<void> {
    this.token?.cancel();
    const token = new FlowToken();
    this.token = token;

    this.setContent(`
      <div class="accounts-step accounts-step-loading" role="status">
        <span class="spinner accounts-spinner-lg" aria-hidden="true"></span>
        <p id="accounts-modal-desc">A pedir código ao GitHub…</p>
      </div>`);

    let start: DeviceCodeStart;
    try {
      start = await accountsApi.deviceStart();
    } catch (e) {
      if (token.cancelled) return;
      this.showError("Não foi possível obter um código do GitHub.", errMessage(e));
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
      <p class="accounts-modal-lead" id="accounts-modal-desc">${who}</p>
      <div class="accounts-code-box">
        <output class="accounts-user-code mono" aria-label="Código de verificação ${esc(start.user_code.split("").join(" "))}">${esc(start.user_code)}</output>
        <button type="button" class="btn" data-modal="copy" data-code="${esc(start.user_code)}">
          <span class="accounts-copy-icon">${icon("copy")}</span><span class="accounts-copy-label">Copiar código</span>
        </button>
      </div>
      <ol class="accounts-steps small">
        <li>Copie o código acima.</li>
        <li>No GitHub (abre-se no browser), cole o código.</li>
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
        const res = await accountsApi.devicePoll(start.client_id, start.device_code);
        if (token.cancelled) return;
        if (res.status === "slow_down") {
          intervalSecs = Math.max(1, intervalSecs, res.interval_secs);
        } else if (res.status === "done") {
          this.onDone(res.account_id);
          this.showSuccess(res.reconnected ? "Conta ligada de novo" : "Conta ligada", res.message, res.warning, token);
          return;
        }
      } catch (e) {
        if (token.cancelled) return;
        this.showError("A autorização não foi concluída.", errMessage(e));
        return;
      }
    }
  }

  private showSuccess(title: string, message: string, warning: string | null, token: FlowToken): void {
    token.cancel();
    this.setContent(
      `
      <div class="accounts-step accounts-step-success" role="status">
        <span class="accounts-result-icon success">${icon("check", 28)}</span>
        <p class="accounts-result-title" id="accounts-modal-desc">${esc(title)}</p>
        <p class="small muted accounts-pre">${esc(message)}</p>
        ${warning ? `<p class="small warn accounts-pre">${esc(warning)}</p>` : ""}
      </div>`,
      '[data-modal="close"]'
    );
    showToast(message || "Conta GitHub ligada.", "ok");
    this.ctx.notifyDataChanged();
    this.closeTimer = window.setTimeout(() => this.close(), warning ? SUCCESS_CLOSE_MS * 3 : SUCCESS_CLOSE_MS);
  }

  private showExpired(token: FlowToken): void {
    token.cancel();
    this.setContent(
      `
      <div class="callout warning" role="alert">
        ${icon("alert")}
        <div class="accounts-callout-body">
          <strong id="accounts-modal-desc">O código expirou.</strong>
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
          <strong id="accounts-modal-desc">${esc(title)}</strong>
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
