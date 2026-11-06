import "./welcome.css";
import { api, type AccountRow } from "../lib/api";
import {
  integrationApi,
  integrationHeadline,
  onIntegrationChanged,
  type CheckState,
  type ComponentStatus,
  type IntegrationStatus,
} from "../lib/api-integration";
import { reposApi, type DiscoveredRepo, type SuggestionReason } from "../lib/api-repos";
import { DeviceFlowModal } from "../lib/device-flow";
import { errMessage, esc, shortenPath, showToast, withBusy } from "../lib/dom";
import { brandSymbol, icon } from "../lib/icons";
import { getOnboarding, setOnboarding, WELCOME_REQUEST_EVENT } from "../lib/onboarding";
import { profileChip, profileName } from "../lib/profile";
import type { MountView, ViewContext } from "../lib/view";
import { mount as mountOverview } from "./overview";

type Step = 1 | 2 | 3;

type Row = { repo: DiscoveredRepo; accountId: string; checked: boolean };

type Discovery =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ok"; truncated: boolean };

type IntegrationState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ok"; value: IntegrationStatus };

type State = {
  loading: boolean;
  step: Step;
  accounts: AccountRow[];
  oauthOk: boolean | null;
  discovery: Discovery;
  rows: Row[];
  bound: number;
  bindErrors: string[];
  binding: boolean;
  integration: IntegrationState;
  applyWarnings: string[];
  applyError: string | null;
};

const OAUTH_UNAVAILABLE = "A ligação ao GitHub não está disponível nesta versão da app.";

const STEPS: { n: Step; eyebrow: string; title: string }[] = [
  { n: 1, eyebrow: "01 / Conta", title: "Ligar a conta GitHub" },
  { n: 2, eyebrow: "02 / Repositórios", title: "Encontrar os seus repositórios" },
  { n: 3, eyebrow: "03 / Pronto", title: "Pronto a usar" },
];

const REASON_TEXT: Record<SuggestionReason, string> = {
  owner_login: "conta dona do repositório",
  same_owner_binding: "já usada noutros repositórios deste dono",
  only_account: "única conta ligada",
  push_access: "única conta com permissão de push",
};

const COMPONENTS: { key: "binaries" | "credentials" | "hooks" | "daemon"; label: string }[] = [
  { key: "binaries", label: "Programas" },
  { key: "credentials", label: "Credenciais Git" },
  { key: "hooks", label: "Verificação nos commits" },
  { key: "daemon", label: "Serviço em segundo plano" },
];

const CHECK_BADGE: Record<CheckState, { cls: string; label: string; sym: "check" | "alert" | "x" }> = {
  ok: { cls: "success", label: "Em ordem", sym: "check" },
  missing: { cls: "warning", label: "Em falta", sym: "alert" },
  outdated: { cls: "warning", label: "Desatualizado", sym: "alert" },
  conflict: { cls: "warning", label: "Conflito", sym: "alert" },
  error: { cls: "error", label: "Erro", sym: "x" },
};

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function repoLabel(r: DiscoveredRepo): string {
  return `${r.github_owner}/${r.github_repo}`;
}

function accountOptionLabel(a: AccountRow): string {
  const name = profileName(a);
  return a.login && a.login !== name ? `${name} (@${a.login})` : name;
}

/* ------------------------------------------------------------------ */
/* Ecrã de boas-vindas                                                 */
/* ------------------------------------------------------------------ */

type WelcomeOptions = {
  /** Contas já lidas pelo chamador (evita um pedido repetido). */
  accounts?: AccountRow[];
  /** Chamado quando o utilizador conclui ou salta o guia. */
  onExit: () => void;
};

export function mountWelcome(root: HTMLElement, ctx: ViewContext, opts: WelcomeOptions): () => void {
  const state: State = {
    loading: true,
    step: 1,
    accounts: opts.accounts ?? [],
    oauthOk: null,
    discovery: { status: "idle" },
    rows: [],
    bound: 0,
    bindErrors: [],
    binding: false,
    integration: { status: "idle" },
    applyWarnings: [],
    applyError: null,
  };

  let disposed = false;
  let discoverSeq = 0;
  let integrationSeq = 0;
  let modal: DeviceFlowModal | null = null;
  /** O foco só pode sair do modal quando este fecha. */
  let deferredFocus: Step | null = null;
  let unlistenIntegration: (() => void) | null = null;

  root.innerHTML = `
    <header class="view-header">
      <div>
        <h1 class="view-title">Boas-vindas</h1>
        <p class="view-sub">Três passos, sem terminal.</p>
      </div>
      <div class="view-actions">
        <button type="button" class="btn ghost" data-action="skip" title="Pode voltar a este guia a partir do Painel">Saltar</button>
      </div>
    </header>
    <p class="sr-only" role="status" aria-live="polite" data-ref="live"></p>
    <div class="welcome">
      <section class="welcome-hero" aria-labelledby="welcome-hero-title">
        <div class="welcome-hero-text">
          <p class="welcome-eyebrow">Boas-vindas</p>
          <h2 class="welcome-hero-title" id="welcome-hero-title">A conta certa em cada projeto.</h2>
          <p class="welcome-hero-sub">Ligue o GitHub, encontre os seus repositórios e o Git passa a usar sozinho a conta e a identidade certas em cada commit e push.</p>
        </div>
        <div class="welcome-hero-mark" aria-hidden="true">${brandSymbol()}</div>
      </section>
      <ol class="welcome-steps" data-ref="steps" aria-label="Passos de configuração"></ol>
    </div>`;

  const live = root.querySelector<HTMLElement>('[data-ref="live"]')!;
  const stepsEl = root.querySelector<HTMLOListElement>('[data-ref="steps"]')!;

  const announce = (msg: string): void => {
    live.textContent = "";
    window.setTimeout(() => {
      if (!disposed) live.textContent = msg;
    }, 50);
  };

  // ---------- Estrutura dos passos ----------

  function stepState(n: Step): "done" | "current" | "locked" {
    if (n < state.step) return "done";
    return n === state.step ? "current" : "locked";
  }

  function headHtml(n: Step): string {
    const def = STEPS[n - 1];
    const st = stepState(n);
    const marker = st === "done" ? icon("check", 16) : `<span aria-hidden="true">${n}</span>`;
    const status =
      st === "done"
        ? `<span class="badge success">${icon("check", 12)}Concluído</span>`
        : st === "locked"
          ? `<span class="muted small">Em seguida</span>`
          : "";
    return `
      <div class="welcome-step-head">
        <span class="welcome-step-marker">${marker}</span>
        <div class="welcome-step-heading">
          <span class="welcome-eyebrow">${esc(def.eyebrow)}</span>
          <h3 class="welcome-step-title" id="welcome-step-${n}-title" tabindex="-1">${esc(def.title)}${st === "done" ? `<span class="sr-only"> (concluído)</span>` : ""}</h3>
        </div>
        ${status}
      </div>`;
  }

  function stepLi(n: Step): HTMLElement {
    return stepsEl.querySelector<HTMLElement>(`[data-step="${n}"]`)!;
  }

  /** Reconstrói a estrutura (cabeçalhos e estados). Usar só quando o passo atual muda. */
  function renderFrame(): void {
    stepsEl.innerHTML = STEPS.map(({ n }) => {
      const st = stepState(n);
      return `<li class="welcome-step is-${st}" data-step="${n}" aria-labelledby="welcome-step-${n}-title"${st === "current" ? ' aria-current="step"' : ""}>
        <div data-ref="head">${headHtml(n)}</div>
        <div class="welcome-step-body-wrap" data-ref="body"></div>
      </li>`;
    }).join("");
    for (const { n } of STEPS) renderBody(n);
  }

  function renderBody(n: Step): void {
    const li = stepsEl.querySelector<HTMLElement>(`[data-step="${n}"]`);
    if (!li) return;
    const body = li.querySelector<HTMLElement>('[data-ref="body"]')!;
    const st = stepState(n);
    if (st === "locked") body.innerHTML = "";
    else if (n === 1) body.innerHTML = body1(st);
    else if (n === 2) body.innerHTML = body2(st);
    else body.innerHTML = body3();
    if (n === 2 && st === "current") updateBindUi();
  }

  function focusHeading(n: Step): void {
    if (modal) {
      deferredFocus = n;
      return;
    }
    stepsEl.querySelector<HTMLElement>(`#welcome-step-${n}-title`)?.focus();
  }

  // ---------- Passo 1 · Conta ----------

  function body1(st: "done" | "current"): string {
    if (state.loading) {
      return `<div class="welcome-step-body"><div class="welcome-loading" role="status"><span class="spinner" aria-hidden="true"></span>A carregar…</div></div>`;
    }
    if (st === "done") {
      const chips = state.accounts.map((a) => `<li>${profileChip(a, { size: "md" })}</li>`).join("");
      return `
        <div class="welcome-step-body welcome-summary">
          <ul class="welcome-chips" aria-label="Contas ligadas">${chips}</ul>
          ${
            state.oauthOk
              ? `<button type="button" class="btn ghost" data-action="connect">${icon("plus", 14)}<span>Ligar outra conta</span></button>`
              : ""
          }
        </div>`;
    }
    const unavailable = state.oauthOk === false;
    return `
      <div class="welcome-step-body">
        <p class="welcome-lead">Ligue a conta GitHub que usa neste Mac. Abre-se uma janela com um código: copie-o, autorize no GitHub e volte aqui. A palavra-passe nunca passa pela app.</p>
        ${
          unavailable
            ? `<div class="callout error" role="alert">${icon("alert", 16)}<span>${esc(OAUTH_UNAVAILABLE)}</span></div>`
            : ""
        }
        <div class="welcome-actions">
          <button type="button" class="btn primary lg welcome-cta" data-action="connect"${unavailable ? " disabled" : ""}>${icon("key", 18)}<span>Ligar conta GitHub</span></button>
        </div>
        <p class="muted small welcome-hint">Pode ligar mais contas depois (por exemplo, pessoal e trabalho).</p>
      </div>`;
  }

  function openConnect(opener: HTMLElement): void {
    if (modal || state.oauthOk === false) return;
    modal = new DeviceFlowModal(
      ctx,
      opener,
      null,
      () => void onAccountConnected(),
      () => {
        modal = null;
        if (disposed) return;
        if (deferredFocus !== null) {
          const n = deferredFocus;
          deferredFocus = null;
          focusHeading(n);
        }
      }
    );
  }

  async function onAccountConnected(): Promise<void> {
    setOnboarding("progress");
    try {
      state.accounts = await api.accounts();
    } catch (e) {
      showToast(errMessage(e));
    }
    if (disposed) return;
    if (state.step === 1) {
      goStep(2);
    } else {
      renderBody(1);
      if (state.step === 2) {
        // Com mais uma conta as sugestões mudam: procura outra vez.
        void startDiscovery();
      }
    }
  }

  // ---------- Passo 2 · Repositórios ----------

  function selectedRows(): Row[] {
    return state.rows.filter((r) => r.checked && r.accountId);
  }

  function accountSelect(i: number, r: Row): string {
    const opts = state.accounts
      .map((a) => `<option value="${esc(a.id)}"${a.id === r.accountId ? " selected" : ""}>${esc(accountOptionLabel(a))}</option>`)
      .join("");
    return `
      <label class="sr-only" for="welcome-acc-${i}">Conta para ${esc(repoLabel(r.repo))}</label>
      <select class="select welcome-select" id="welcome-acc-${i}" data-row="${i}" data-kind="account">
        <option value=""${r.accountId ? "" : " selected"}>Escolher conta…</option>
        ${opts}
      </select>`;
  }

  function rowHtml(i: number, r: Row): string {
    const acc = state.accounts.find((a) => a.id === r.accountId);
    const reason =
      acc && r.repo.suggested_account_id === r.accountId && r.repo.suggestion_reason
        ? `<span class="muted small">Sugerida: ${esc(REASON_TEXT[r.repo.suggestion_reason])}</span>`
        : "";
    const accountCell =
      state.accounts.length > 1
        ? accountSelect(i, r)
        : acc
          ? profileChip(acc)
          : "";
    return `
      <li class="welcome-repo">
        <input type="checkbox" class="welcome-check" id="welcome-sel-${i}" data-row="${i}" data-kind="check"${r.checked ? " checked" : ""}${r.accountId ? "" : " disabled"} aria-describedby="welcome-path-${i}" />
        <label class="welcome-repo-text" for="welcome-sel-${i}">
          <span class="mono welcome-repo-name truncate">${esc(repoLabel(r.repo))}</span>
          <span class="mono muted small truncate" id="welcome-path-${i}" title="${esc(r.repo.root_path)}">${esc(shortenPath(r.repo.root_path))}</span>
        </label>
        <span class="welcome-repo-account">${accountCell}${reason}</span>
      </li>`;
  }

  function body2(st: "done" | "current"): string {
    if (st === "done") {
      const text =
        state.bound > 0
          ? `${plural(state.bound, "repositório vinculado", "repositórios vinculados")}. O Git já usa a conta certa neles.`
          : "Nenhum repositório vinculado por agora. Pode fazê-lo mais tarde em Repositórios.";
      return `<div class="welcome-step-body"><p class="welcome-lead">${esc(text)}</p></div>`;
    }

    const d = state.discovery;
    if (d.status === "idle" || d.status === "loading") {
      return `
        <div class="welcome-step-body">
          <div class="welcome-loading" role="status"><span class="spinner" aria-hidden="true"></span>A procurar clones do GitHub nas pastas habituais…</div>
          <div class="welcome-sk-list" aria-hidden="true">${Array.from({ length: 3 }, () => `<div class="skeleton welcome-sk-row"></div>`).join("")}</div>
        </div>`;
    }
    if (d.status === "error") {
      return `
        <div class="welcome-step-body">
          <div class="callout error" role="alert">${icon("alert", 16)}<span>Não foi possível procurar repositórios.\n${esc(d.message)}</span></div>
          <div class="welcome-actions">
            <button type="button" class="btn primary lg" data-action="retry-discover">${icon("refresh", 16)}<span>Tentar de novo</span></button>
            ${cloudBtn()}
            <button type="button" class="btn ghost lg" data-action="next">Saltar este passo</button>
          </div>
        </div>`;
    }

    const errors = state.bindErrors.length
      ? `<div class="callout error" role="alert">${icon("alert", 16)}<span>${esc(`Alguns repositórios não foram vinculados:\n${state.bindErrors.join("\n")}`)}</span></div>`
      : "";
    const skipLabel = state.bound > 0 ? "Continuar" : "Saltar este passo";

    if (state.rows.length === 0) {
      return `
        <div class="welcome-step-body">
          ${errors}
          <div class="empty-state welcome-empty">
            <span class="empty-icon">${icon("repo", 24)}</span>
            <h4 class="welcome-empty-title">${state.bound > 0 ? "Não há mais repositórios por vincular" : "Não encontrámos clones por vincular"}</h4>
            <p>Procurámos nas pastas habituais. Se os seus projetos estão noutro sítio, escolha a pasta; se ainda não os tem no Mac, clone-os a partir do GitHub.</p>
          </div>
          <div class="welcome-actions">
            <button type="button" class="btn lg" data-action="browse">${icon("folder", 16)}<span>Procurar noutra pasta</span></button>
            ${cloudBtn()}
            <button type="button" class="btn primary lg" data-action="next">${skipLabel}</button>
          </div>
        </div>`;
    }

    const rows = state.rows.map((r, i) => rowHtml(i, r)).join("");
    return `
      <div class="welcome-step-body">
        <p class="welcome-lead">Encontrámos <strong>${plural(state.rows.length, "repositório", "repositórios")}</strong> do GitHub neste Mac sem conta associada. Escolha os que quer vincular: o commit e o push passam a usar a conta indicada.</p>
        ${errors}
        <div class="welcome-toolbar">
          <button type="button" class="btn ghost" data-action="toggle-all"></button>
          <span class="small muted" data-ref="count" role="status" aria-live="polite"></span>
        </div>
        <ul class="list welcome-repos" data-ref="repos">${rows}</ul>
        ${
          d.truncated
            ? `<p class="muted small welcome-hint">A procura foi limitada no tempo e pode haver mais repositórios. Use "Procurar noutra pasta" ou veja Repositórios mais tarde.</p>`
            : ""
        }
        <div class="welcome-actions">
          <button type="button" class="btn primary lg" data-action="bind"></button>
          <button type="button" class="btn lg" data-action="browse">${icon("folder", 16)}<span>Procurar noutra pasta</span></button>
          ${cloudBtn()}
          <button type="button" class="btn ghost lg" data-action="next">${skipLabel}</button>
        </div>
      </div>`;
  }

  function cloudBtn(): string {
    return `<button type="button" class="btn lg" data-action="cloud">${icon("cloud", 16)}<span>Clonar do GitHub</span></button>`;
  }

  /** Atualiza contagem, botão de vincular e "selecionar todos" sem reconstruir a lista. */
  function updateBindUi(): void {
    const li = stepLi(2);
    const bind = li.querySelector<HTMLButtonElement>('[data-action="bind"]');
    if (!bind) return;
    const toggle = li.querySelector<HTMLButtonElement>('[data-action="toggle-all"]')!;
    const count = li.querySelector<HTMLElement>('[data-ref="count"]')!;
    const n = selectedRows().length;
    const eligible = state.rows.filter((r) => r.accountId).length;
    const all = n > 0 && n === state.rows.length;
    bind.disabled = n === 0 || state.binding;
    bind.innerHTML = `${icon("check", 16)}<span>${all ? `Vincular todos (${n})` : n > 0 ? `Vincular selecionados (${n})` : "Vincular selecionados"}</span>`;
    toggle.textContent = n === eligible && eligible > 0 ? "Limpar seleção" : "Selecionar todos";
    toggle.disabled = eligible === 0;
    const missing = state.rows.length - eligible;
    count.textContent =
      `${n} de ${state.rows.length} selecionados` + (missing > 0 ? ` · ${plural(missing, "sem conta escolhida", "sem conta escolhida")}` : "");
  }

  async function startDiscovery(extraRoot?: string): Promise<void> {
    const seq = ++discoverSeq;
    state.discovery = { status: "loading" };
    if (!extraRoot) state.rows = [];
    renderBody(2);
    try {
      const res = await reposApi.discover(extraRoot ? [extraRoot] : null);
      if (disposed || seq !== discoverSeq) return;
      const known = new Set(state.rows.map((r) => r.repo.root_path));
      const single = state.accounts.length === 1 ? state.accounts[0].id : "";
      for (const repo of res.repos) {
        if (known.has(repo.root_path)) continue;
        const accountId = repo.suggested_account_id ?? single;
        state.rows.push({ repo, accountId, checked: accountId !== "" });
      }
      state.discovery = { status: "ok", truncated: res.truncated };
      renderBody(2);
      announce(
        state.rows.length
          ? `Encontrados ${plural(state.rows.length, "repositório", "repositórios")}.`
          : "Não foram encontrados repositórios por vincular."
      );
    } catch (e) {
      if (disposed || seq !== discoverSeq) return;
      state.discovery = { status: "error", message: errMessage(e) };
      renderBody(2);
    }
  }

  async function browse(): Promise<void> {
    let folder: string | null;
    try {
      folder = await reposApi.pickFolder();
    } catch (e) {
      showToast(errMessage(e));
      return;
    }
    if (disposed || !folder) return;
    focusHeading(2);
    await startDiscovery(folder);
  }

  async function bindSelected(btn: HTMLButtonElement): Promise<void> {
    const chosen = selectedRows();
    if (!chosen.length || state.binding) return;
    state.binding = true;
    const list = stepLi(2).querySelector<HTMLElement>('[data-ref="repos"]');
    list?.setAttribute("inert", "");
    try {
      const results = await withBusy(btn, "A vincular…", () =>
        reposApi.bindMany(chosen.map((r) => ({ path: r.repo.root_path, account_id: r.accountId })))
      );
      if (disposed) return;
      const failedPaths = new Set(results.filter((r) => !r.repo).map((r) => r.path));
      const okCount = results.length - failedPaths.size;
      state.bound += okCount;
      state.bindErrors = results
        .filter((r) => r.error)
        .map((r) => `${shortenPath(r.path)}: ${r.error}`);
      state.rows = state.rows.filter((r) => !chosen.includes(r) || failedPaths.has(r.repo.root_path));
      if (okCount > 0) ctx.notifyDataChanged();
      if (state.bindErrors.length === 0) {
        showToast(`${plural(okCount, "repositório vinculado", "repositórios vinculados")}.`, "ok");
        goStep(3);
      } else {
        state.binding = false;
        renderBody(2);
        announce("Alguns repositórios não foram vinculados.");
      }
    } catch (e) {
      showToast(errMessage(e));
    } finally {
      state.binding = false;
      list?.removeAttribute("inert");
      if (!disposed && state.step === 2) updateBindUi();
    }
  }

  // ---------- Passo 3 · Integração ----------

  function componentRow(label: string, c: ComponentStatus): string {
    const b = CHECK_BADGE[c.state];
    return `
      <li class="welcome-check-row">
        <span class="welcome-check-label">${esc(label)}</span>
        <span class="badge ${b.cls}">${icon(b.sym, 12)}${esc(b.label)}</span>
        <span class="muted small welcome-check-summary">${esc(c.summary)}</span>
      </li>`;
  }

  function body3(): string {
    const it = state.integration;
    if (it.status === "idle" || it.status === "loading") {
      return `<div class="welcome-step-body"><div class="welcome-loading" role="status"><span class="spinner" aria-hidden="true"></span>A verificar a integração com o Git…</div></div>`;
    }
    if (it.status === "error") {
      return `
        <div class="welcome-step-body">
          <div class="callout error" role="alert">${icon("alert", 16)}<span>Não foi possível verificar a integração com o Git.\n${esc(it.message)}</span></div>
          <div class="welcome-actions">
            <button type="button" class="btn lg" data-action="retry-integration">${icon("refresh", 16)}<span>Tentar de novo</span></button>
            <button type="button" class="btn primary lg" data-action="finish">Ir para o Painel</button>
          </div>
        </div>`;
    }

    const st = it.value;
    const head = integrationHeadline(st);
    const headIcon = head.kind === "success" ? "check" : head.kind === "warning" ? "alert" : "x";
    const hasConflict = [st.credentials, st.hooks].some((c) => c.state === "conflict");
    const warnings = state.applyWarnings.length
      ? `<div class="callout warning" role="status">${icon("alert", 16)}<span>${esc(state.applyWarnings.join("\n"))}</span></div>`
      : "";
    const applyErr = state.applyError
      ? `<div class="callout error" role="alert">${icon("alert", 16)}<span>${esc(`Não foi possível ativar a integração.\n${state.applyError}`)}</span></div>`
      : "";
    const gitMissing = !st.git_found
      ? `<div class="callout error" role="alert">${icon("alert", 16)}<span>O Git não foi encontrado neste Mac. Instale-o (ferramentas de programação da Apple ou git-scm.com) e volte a verificar.</span></div>`
      : "";

    const verdict = st.ready
      ? `<div class="callout success" role="status">${icon("check", 16)}<span>Tudo pronto. A partir de agora, o <span class="mono">git commit</span> e o <span class="mono">git push</span> usam a conta certa em cada repositório vinculado.</span></div>`
      : `<div class="callout warning" role="status">${icon("alert", 16)}<span>${
          st.enabled
            ? "Falta ativar a integração para o Git usar a conta certa automaticamente. É um clique e pode ser desfeito em Definições."
            : "A integração está desativada. Ative-a para o Git usar a conta certa automaticamente."
        }</span></div>`;

    const actions = st.ready
      ? `<button type="button" class="btn primary lg welcome-cta" data-action="finish">Ir para o Painel</button>`
      : `
        <button type="button" class="btn primary lg welcome-cta" data-action="apply"${st.git_found ? "" : " disabled"}>${icon("check", 18)}<span>Ativar</span></button>
        ${hasConflict ? `<button type="button" class="btn lg" data-action="settings">Rever em Definições</button>` : ""}
        <button type="button" class="btn ghost lg" data-action="finish">Ir para o Painel</button>`;

    return `
      <div class="welcome-step-body">
        <div class="welcome-integration-head">
          <span class="badge ${head.kind}">${icon(headIcon, 12)}Integração Git · ${esc(head.label)}</span>
          <span class="muted small">${esc(head.detail)}</span>
        </div>
        ${gitMissing}${verdict}${warnings}${applyErr}
        <ul class="list welcome-checks" aria-label="Estado de cada componente">${COMPONENTS.map((c) => componentRow(c.label, st[c.key])).join("")}</ul>
        <div class="welcome-actions">${actions}</div>
      </div>`;
  }

  async function loadIntegration(): Promise<void> {
    const seq = ++integrationSeq;
    state.integration = { status: "loading" };
    renderBody(3);
    try {
      const value = await integrationApi.status();
      if (disposed || seq !== integrationSeq) return;
      state.integration = { status: "ok", value };
    } catch (e) {
      if (disposed || seq !== integrationSeq) return;
      state.integration = { status: "error", message: errMessage(e) };
    }
    renderBody(3);
    if (state.integration.status === "ok") {
      announce(state.integration.value.ready ? "A integração com o Git está ativa." : "A integração com o Git precisa de ser ativada.");
    }
  }

  async function applyIntegration(btn: HTMLButtonElement): Promise<void> {
    state.applyError = null;
    state.applyWarnings = [];
    try {
      const report = await withBusy(btn, "A ativar…", () => integrationApi.apply(false));
      if (disposed) return;
      integrationSeq++;
      state.integration = { status: "ok", value: report.status };
      state.applyWarnings = report.warnings;
      renderBody(3);
      focusHeading(3);
      announce(report.status.ready ? "Integração com o Git ativada." : "A integração ficou por completar. Veja os avisos.");
    } catch (e) {
      if (disposed) return;
      state.applyError = errMessage(e);
      renderBody(3);
    }
  }

  // ---------- Navegação entre passos ----------

  function goStep(n: Step): void {
    state.step = n;
    renderFrame();
    focusHeading(n);
    announce(`Passo ${n} de 3: ${STEPS[n - 1].title}.`);
    if (n === 2) void startDiscovery();
    if (n === 3) void loadIntegration();
  }

  function exit(kind: "done" | "skipped"): void {
    setOnboarding(kind);
    opts.onExit();
  }

  // ---------- Eventos ----------

  const onClick = (ev: Event): void => {
    const el = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
    if (!el || el.disabled || !root.contains(el)) return;
    switch (el.dataset.action) {
      case "skip":
        exit("skipped");
        break;
      case "connect":
        openConnect(el);
        break;
      case "retry-discover":
        focusHeading(2);
        void startDiscovery();
        break;
      case "browse":
        void browse();
        break;
      case "cloud":
        ctx.navigate("cloud");
        break;
      case "next":
        goStep(3);
        break;
      case "toggle-all": {
        const eligible = state.rows.filter((r) => r.accountId);
        const allOn = eligible.length > 0 && eligible.every((r) => r.checked);
        for (const r of eligible) r.checked = !allOn;
        for (const [i, r] of state.rows.entries()) {
          const cb = stepsEl.querySelector<HTMLInputElement>(`#welcome-sel-${i}`);
          if (cb) cb.checked = r.checked;
        }
        updateBindUi();
        break;
      }
      case "bind":
        void bindSelected(el);
        break;
      case "retry-integration":
        void loadIntegration();
        break;
      case "apply":
        void applyIntegration(el);
        break;
      case "settings":
        ctx.navigate("settings");
        break;
      case "finish":
        exit("done");
        break;
    }
  };

  const onChange = (ev: Event): void => {
    const el = ev.target as HTMLInputElement | HTMLSelectElement;
    const idx = el.dataset?.row;
    if (idx === undefined) return;
    const row = state.rows[Number(idx)];
    if (!row) return;
    if (el.dataset.kind === "check") {
      row.checked = (el as HTMLInputElement).checked;
    } else if (el.dataset.kind === "account") {
      row.accountId = (el as HTMLSelectElement).value;
      row.checked = row.accountId !== "";
      const cb = stepsEl.querySelector<HTMLInputElement>(`#welcome-sel-${idx}`);
      if (cb) {
        cb.disabled = row.accountId === "";
        cb.checked = row.checked;
      }
    }
    updateBindUi();
  };

  root.addEventListener("click", onClick);
  root.addEventListener("change", onChange);

  void onIntegrationChanged((value) => {
    if (disposed || state.integration.status !== "ok") return;
    if (JSON.stringify(value) === JSON.stringify(state.integration.value)) return;
    state.integration = { status: "ok", value };
    if (state.step === 3) renderBody(3);
  }).then((un) => {
    if (disposed) un();
    else unlistenIntegration = un;
  });

  // ---------- Arranque ----------

  renderFrame();
  void (async () => {
    const [oauth, accounts] = await Promise.allSettled([
      api.oauthStatus(),
      opts.accounts ? Promise.resolve(opts.accounts) : api.accounts(),
    ]);
    if (disposed) return;
    state.oauthOk = oauth.status === "fulfilled" ? oauth.value.ok : false;
    if (accounts.status === "fulfilled") state.accounts = accounts.value;
    state.loading = false;
    state.step = state.accounts.length > 0 ? 2 : 1;
    renderFrame();
    if (state.step === 2) void startDiscovery();
    if (accounts.status === "rejected") showToast(errMessage(accounts.reason));
    // Foco no primeiro controlo útil sem roubar o foco se o utilizador já navegou.
    if (!root.contains(document.activeElement) || document.activeElement === document.body) focusHeading(state.step);
  })();

  return () => {
    disposed = true;
    root.removeEventListener("click", onClick);
    root.removeEventListener("change", onChange);
    unlistenIntegration?.();
    modal?.close();
  };
}

/* ------------------------------------------------------------------ */
/* Início: guia de boas-vindas no primeiro arranque, depois o Painel    */
/* ------------------------------------------------------------------ */

/**
 * Vista "Painel": no primeiro arranque (sem contas e sem o guia concluído/saltado) mostra as
 * boas-vindas; caso contrário, o Painel normal.
 */
export const mountHome: MountView = (root, ctx) => {
  let disposed = false;
  let inner: (() => void) | void;

  const teardown = (): void => {
    if (typeof inner === "function") inner();
    inner = undefined;
  };

  const showOverview = (focus = false): void => {
    teardown();
    root.innerHTML = "";
    root.scrollTop = 0;
    inner = mountOverview(root, ctx);
    if (focus) root.focus({ preventScroll: true });
  };

  const showWelcome = (accounts?: AccountRow[]): void => {
    teardown();
    root.innerHTML = "";
    root.scrollTop = 0;
    inner = mountWelcome(root, ctx, { accounts, onExit: () => showOverview(true) });
  };

  const onRequest = (): void => showWelcome();
  document.addEventListener(WELCOME_REQUEST_EVENT, onRequest);

  const flag = getOnboarding();
  if (flag === "done" || flag === "skipped") {
    showOverview();
  } else {
    root.innerHTML = `<div class="welcome-boot" role="status" aria-live="polite"><span class="spinner" aria-hidden="true"></span><span class="sr-only">A carregar…</span></div>`;
    api.accounts().then(
      (accounts) => {
        if (disposed) return;
        if (flag === "progress" || accounts.length === 0) {
          showWelcome(accounts);
        } else {
          // Quem já tinha contas antes do guia existir não precisa dele.
          setOnboarding("done");
          showOverview();
        }
      },
      () => {
        if (!disposed) showOverview();
      }
    );
  }

  return () => {
    disposed = true;
    document.removeEventListener(WELCOME_REQUEST_EVENT, onRequest);
    teardown();
  };
};
