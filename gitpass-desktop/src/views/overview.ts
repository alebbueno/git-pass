import "./overview.css";
import { api, type AccountRow, type BindingRow } from "../lib/api";
import { integrationApi, integrationHeadline, onIntegrationChanged, type IntegrationStatus } from "../lib/api-integration";
import { reposApi, type RepoRow } from "../lib/api-repos";
import { systemApi, type AuditEntry, type CurrentRepoState, type Diagnostics, type EmergencyState } from "../lib/api-system";
import { errMessage, esc, shortenPath, showToast, withBusy } from "../lib/dom";
import { codeChip, errorView, parseError } from "../lib/errors";
import { icon, type IconName } from "../lib/icons";
import { getOnboarding, WELCOME_REQUEST_EVENT } from "../lib/onboarding";
import { profileChip } from "../lib/profile";
import {
  announceDiagnostics,
  currentRepoName,
  DIAGNOSTICS_UPDATED_EVENT,
  onSystemState,
  pinRepo,
  SOURCE_META,
  verdictMeta,
  type Loadable,
} from "../lib/system-state";
import { DATA_CHANGED_EVENT, DRAWER_EVENT, type DrawerEventDetail, type MountView, type Route, type ViewId } from "../lib/view";
import { repoAvatar, sourceTag } from "../shell/current-repo";
import { auditRow } from "./integration";
import { drawerRoute } from "./repo-detail";
import { startFolderMap } from "./repos-map";
import { columnsHtml, plural, projectName, projectRowHtml, reviewReasons } from "./repos-ui";

const MAX_RECENT = 4;
const MAX_TO_SEND = 5;
const MAX_EVENTS = 5;

type State = {
  oauth: Loadable<boolean>;
  accounts: Loadable<AccountRow[]>;
  bindings: Loadable<BindingRow[]>;
  integration: Loadable<IntegrationStatus>;
  repos: Loadable<RepoRow[]>;
  diagnostics: Loadable<Diagnostics>;
  audit: Loadable<AuditEntry[]>;
  current: Loadable<CurrentRepoState>;
  emergency: Loadable<EmergencyState>;
};

const currentState = (s: State): CurrentRepoState | null => (s.current.status === "ok" ? s.current.value : null);

function stat(ic: IconName, label: string, value: string, hint: string, busy = false): string {
  return `<section class="surface stat"${busy ? ' aria-busy="true"' : ""}>
    <div class="stat-label">${icon(ic)}${esc(label)}</div>
    <div class="stat-value">${busy ? `<span class="skeleton home-stat-sk" aria-hidden="true"></span><span class="sr-only">Carregando…</span>` : esc(value)}</div>
    <p class="stat-hint">${esc(hint)}</p>
  </section>`;
}

function sectionHead(id: string, title: string, link?: { route: Route; label: string }): string {
  return `<div class="section-head"><h2 class="section-title" id="${id}">${esc(title)}</h2>${
    link ? `<button type="button" class="control ghost" data-nav="${esc(link.route)}">${esc(link.label)}${icon("chevronRight")}</button>` : ""
  }</div>`;
}

function renderStats(s: State): string {
  const accBusy = s.accounts.status === "loading";
  const repoBusy = s.repos.status === "loading";
  const accounts = s.accounts.status === "ok" ? s.accounts.value : [];
  const repos = s.repos.status === "ok" ? s.repos.value : [];
  const off = accounts.filter((a) => !a.has_token).length;
  const review = repos.filter((r) => reviewReasons(r, accounts, currentState(s)).length > 0).length;
  const projects =
    s.repos.status === "error" ? stat("repo", "Projetos vinculados", "—", "Não foi possível ler") : stat("repo", "Projetos vinculados", String(repos.length), "Repositórios locais gerenciados", repoBusy);
  const accs =
    s.accounts.status === "error"
      ? stat("user", "Contas conectadas", "—", "Não foi possível ler")
      : stat(
          "user",
          "Contas conectadas",
          String(accounts.length - off),
          off ? plural(off, "conta desconectada", "contas desconectadas") : accounts.length ? "Perfis de GitHub disponíveis" : "Nenhuma conta ainda",
          accBusy
        );
  const ids =
    s.repos.status === "error"
      ? stat("shield", "Identidades corretas", "—", "Não foi possível ler")
      : stat(
          "shield",
          "Identidades corretas",
          `${repos.length - review} / ${repos.length}`,
          review ? (review === 1 ? "Uma identidade para revisar" : `${review} identidades para revisar`) : repos.length ? "Tudo em dia" : "Nenhum projeto ainda",
          repoBusy || accBusy
        );
  return `<div class="stats">${projects}${accs}${ids}</div>`;
}

// ---------- Avisos ----------

function renderAttention(s: State): string {
  const out: string[] = [];
  if (s.oauth.status === "ok" && !s.oauth.value) {
    out.push(`<div class="home-attention error" role="alert">${icon("alert")}<span><strong>Esta versão do app não tem o GitHub configurado.</strong> Não dá para conectar contas novas. Atualize o GitPass.</span></div>`);
  }
  if (s.integration.status === "ok") {
    const h = integrationHeadline(s.integration.value);
    if (h.kind !== "success") {
      const text = s.integration.value.enabled
        ? "A integração com o Git está com problema: os commits podem não usar a conta certa."
        : "A integração com o Git está desligada: o Git não está usando o GitPass.";
      out.push(`<div class="home-attention${h.kind === "error" ? " error" : ""}" role="status">${icon("plug")}<span>${esc(text)}</span>
        <button type="button" class="control" data-nav="integration">${icon("chevronRight")}Abrir Integração</button></div>`);
    }
  }
  if (s.repos.status === "ok" && s.accounts.status === "ok") {
    const st = currentState(s);
    const review = s.repos.value.filter((r) => reviewReasons(r, s.accounts.status === "ok" ? s.accounts.value : [], st).length > 0);
    const mismatched = s.repos.value.filter((r) => r.exists && r.identity.state === "different");
    if (review.length) {
      const text =
        mismatched.length === review.length
          ? mismatched.length === 1
            ? "Um projeto está com a identidade local diferente da conta vinculada."
            : `${mismatched.length} projetos estão com a identidade local diferente da conta vinculada.`
          : review.length === 1
            ? "Um projeto precisa de revisão."
            : `${review.length} projetos precisam de revisão.`;
      out.push(`<div class="home-attention" role="status">${icon("alert")}<span>${esc(text)}</span>
        ${mismatched.length > 1 ? `<button type="button" class="control ghost" data-action="fix-all">${icon("check")}Corrigir todos</button>` : ""}
        <button type="button" class="control" data-nav="repos?review=1">${icon("chevronRight")}Revisar identidade</button></div>`);
    }
  }
  return out.join("");
}

function step(n: string, title: string, desc: string, done: boolean, next: boolean, view: ViewId): string {
  return `<li>
    <button type="button" class="home-step${done ? " done" : ""}${next ? " next" : ""}" data-nav="${view}">
      <span class="home-step-marker" aria-hidden="true">${done ? icon("check") : esc(n)}</span>
      <span class="home-step-text"><strong>${esc(title)}</strong><span class="caption muted">${esc(desc)}</span></span>
      ${done ? `<span class="state">${icon("check")}Pronto</span>` : `<span class="home-step-go">${next ? "Começar" : "Abrir"}${icon("chevronRight")}</span>`}
    </button>
  </li>`;
}

function renderOnboarding(accounts: AccountRow[], bindings: BindingRow[]): string {
  if (accounts.length > 0 && bindings.length > 0) return "";
  const hasAcc = accounts.length > 0;
  const hasBind = bindings.length > 0;
  return `<section class="surface home-onboarding" aria-labelledby="home-onboarding-title">
    <div class="section-head"><h2 class="section-title" id="home-onboarding-title">Primeiros passos</h2>${
      getOnboarding() === "skipped" ? `<button type="button" class="control ghost" data-action="welcome">Abrir guia de boas-vindas</button>` : ""
    }</div>
    <p class="caption muted">Duas etapas até o Git usar sempre a identidade certa.</p>
    <ol class="home-steps">
      ${step("1", "Conectar conta do GitHub", "Autorize uma conta para criar um perfil.", hasAcc, !hasAcc, "accounts")}
      ${step("2", "Vincular um repositório", "Escolha uma pasta e vincule a um perfil.", hasBind, hasAcc && !hasBind, "repos")}
    </ol>
    ${
      hasAcc && !hasBind
        ? `<div class="row-actions"><button type="button" class="control primary" data-action="map-folder">${icon("folder")}Mapear pasta…</button></div>`
        : ""
    }
  </section>`;
}

// ---------- Repositórios recentes ----------

function recentOrder(repos: RepoRow[], st: CurrentRepoState | null): RepoRow[] {
  const rank = new Map<string, number>();
  if (st?.current?.binding_id) rank.set(st.current.binding_id, 0);
  st?.recent.forEach((r, i) => {
    if (r.binding_id && !rank.has(r.binding_id)) rank.set(r.binding_id, i + 1);
  });
  return [...repos].sort((a, b) => {
    const ra = rank.get(a.id) ?? Number.MAX_SAFE_INTEGER;
    const rb = rank.get(b.id) ?? Number.MAX_SAFE_INTEGER;
    return ra - rb || (b.sync?.ahead ?? 0) - (a.sync?.ahead ?? 0) || projectName(a).localeCompare(projectName(b), "pt-BR", { sensitivity: "base" });
  });
}

function renderRecent(s: State, accounts: AccountRow[], expanded: string | null): string {
  const head = sectionHead("home-recent-title", "Repositórios recentes", { route: "repos", label: "Ver todos" });
  if (s.repos.status === "loading") {
    return `<section class="page-section home-recent" aria-labelledby="home-recent-title" aria-busy="true">${head}
      <div class="list-frame">${columnsHtml()}<ul class="projects">${Array.from({ length: 3 }, () => `<li class="home-row-sk"><span class="skeleton"></span></li>`).join("")}</ul></div></section>`;
  }
  if (s.repos.status === "error") {
    return `<section class="page-section home-recent" aria-labelledby="home-recent-title">${head}${errorView({
      title: "Não foi possível ler os repositórios",
      error: s.repos.error,
      compact: true,
      actions: [{ label: "Tentar de novo", action: "reload", icon: "refresh" }],
    })}</section>`;
  }
  if (s.repos.value.length === 0) {
    return `<section class="page-section home-recent" aria-labelledby="home-recent-title">${head}
      <div class="list-frame empty"><strong>Nenhum repositório vinculado</strong>Vincule uma pasta com um repositório do GitHub para que o commit use o perfil certo.
        ${
          accounts.length
            ? `<div class="row-actions home-empty-actions"><button type="button" class="control primary" data-action="map-folder">${icon("folder")}Mapear pasta…</button><button type="button" class="control" data-nav="repos">${icon("plus")}Vincular repositório</button></div>`
            : ""
        }</div></section>`;
  }
  const list = recentOrder(s.repos.value, currentState(s)).slice(0, MAX_RECENT);
  return `<section class="page-section home-recent" aria-labelledby="home-recent-title">${head}
    <div class="list-frame">${columnsHtml()}<ul class="projects" aria-label="Repositórios recentes">${list
      .map((r) => projectRowHtml(r, accounts, currentState(s), { expanded: r.id === expanded }))
      .join("")}</ul></div></section>`;
}

// ---------- Repo atual ----------

function renderCurrent(s: State, accounts: AccountRow[]): string {
  const head = sectionHead("home-current-title", "Repo atual");
  const c = s.current;
  if (c.status === "loading") {
    return `<section class="page-section" aria-labelledby="home-current-title" aria-busy="true">${head}<div class="surface home-current"><span class="skeleton home-current-sk"></span></div></section>`;
  }
  if (c.status === "error") {
    const p = parseError(c.error);
    return `<section class="page-section" aria-labelledby="home-current-title">${head}<div class="surface home-current">
      <div class="home-current-main"><strong>Não foi possível saber o repo atual</strong><p class="caption muted">${esc(p.cause)}</p>${p.code ? codeChip(p.code) : ""}</div>
    </div></section>`;
  }
  const st = c.value;
  const cur = st.current;
  if (!cur) {
    return `<section class="page-section" aria-labelledby="home-current-title">${head}<div class="surface home-current">
      <span class="avatar neutral" style="--size:38px" aria-hidden="true">${icon("repo")}</span>
      <div class="home-current-main"><strong>Nenhum repo atual ainda</strong>
        <p class="caption muted">Quando você fizer um commit num repositório vinculado, ele aparece aqui com o resultado da verificação. Você também pode fixar um repositório.</p></div>
    </div></section>`;
  }
  const v = verdictMeta(cur.verdict);
  const acc = cur.account_id ? accounts.find((a) => a.id === cur.account_id) : undefined;
  const pinned = cur.binding_id !== null && st.pinned_binding_id === cur.binding_id;
  const accChip = acc
    ? profileChip(acc)
    : cur.account_id
      ? `<span class="state error">${icon("x")}Conta removida</span>`
      : `<span class="state neutral">${icon("help")}Sem conta vinculada</span>`;
  const tone = v.kind === "success" ? "" : v.kind === "neutral" ? " neutral" : ` ${v.kind}`;
  return `<section class="page-section" aria-labelledby="home-current-title">${head}<div class="surface home-current">
    ${repoAvatar(cur, accounts, 38)}
    <div class="home-current-main">
      <div class="home-current-title"><strong class="truncate">${esc(currentRepoName(cur))}</strong>${sourceTag(cur)}</div>
      <code class="truncate" title="${esc(cur.root_path)}">${esc(shortenPath(cur.root_path))}</code>
      <div class="home-current-facts">${accChip}<span class="state${tone}" title="${esc(v.detail)}">${icon(v.kind === "success" ? "check" : v.kind === "error" ? "xCircle" : "alert")}${esc(v.label)}</span></div>
      <p class="caption muted">${esc(cur.message || v.detail)}${cur.code ? ` ${codeChip(cur.code)}` : ""}</p>
    </div>
    <div class="home-current-actions">${
      cur.binding_id
        ? `<button type="button" class="control" data-action="${pinned ? "unpin" : "pin"}" data-id="${esc(cur.binding_id)}">${icon(pinned ? "pinOff" : "pin")}${pinned ? "Desafixar" : "Fixar"}</button>
           <button type="button" class="control primary" data-nav="${esc(drawerRoute(cur.binding_id))}">Abrir${icon("chevronRight")}</button>`
        : `<button type="button" class="control" data-nav="repos">${icon("plus")}Vincular</button>`
    }</div>
    <span class="sr-only">${esc(SOURCE_META[cur.source].detail)}</span>
  </div></section>`;
}

// ---------- Commits para enviar ----------

function renderToSend(s: State, accounts: AccountRow[]): string {
  if (s.repos.status !== "ok" || s.repos.value.length === 0) return "";
  const pending = s.repos.value.filter((r) => r.exists && (r.sync?.ahead ?? 0) > 0).sort((a, b) => (b.sync?.ahead ?? 0) - (a.sync?.ahead ?? 0));
  const head = sectionHead("home-send-title", pending.length ? `Commits para enviar · ${pending.length}` : "Commits para enviar", pending.length > MAX_TO_SEND ? { route: "repos?ahead=1", label: "Ver todos" } : undefined);
  if (!pending.length) {
    return `<section class="page-section" aria-labelledby="home-send-title">${head}<div class="surface home-note">${icon("checkCircle")}<span>Tudo enviado. Nenhum dos ${esc(
      plural(s.repos.value.length, "repositório", "repositórios")
    )} tem commits locais esperando.</span></div></section>`;
  }
  const rows = pending
    .slice(0, MAX_TO_SEND)
    .map((r) => {
      const acc = accounts.find((a) => a.id === r.account_id);
      const ahead = r.sync?.ahead ?? 0;
      return `<li><button type="button" class="home-send-row" data-nav="${esc(drawerRoute(r.id, "push"))}">
        <span class="home-send-count" aria-hidden="true">${icon("arrowUp", 12)}${ahead}</span>
        <span class="home-send-text"><strong class="truncate">${esc(projectName(r))}</strong><span class="caption muted truncate">${
          r.branch ? `<code>${esc(r.branch)}</code> · ` : ""
        }${esc(plural(ahead, "commit para enviar", "commits para enviar"))}${r.sync?.has_upstream ? "" : " · branch novo"}</span></span>
        ${acc ? profileChip(acc) : `<span class="state error">${icon("x")}Conta removida</span>`}
        <span class="project-chevron" aria-hidden="true">${icon("chevronRight")}</span>
      </button></li>`;
    })
    .join("");
  return `<section class="page-section" aria-labelledby="home-send-title">${head}<ul class="surface home-send">${rows}</ul></section>`;
}

// ---------- Bloqueios ----------

function renderEvents(s: State): string {
  const head = sectionHead("home-events-title", "Bloqueios e não confirmados", { route: "integration", label: "Auditoria" });
  if (s.audit.status === "loading") {
    return `<section class="page-section" aria-labelledby="home-events-title" aria-busy="true">${head}<div class="surface home-note"><span class="skeleton home-events-sk"></span></div></section>`;
  }
  if (s.audit.status === "error") {
    const p = parseError(s.audit.error);
    return `<section class="page-section" aria-labelledby="home-events-title">${head}<div class="surface home-note">${icon("info")}<span>${esc(
      p.unavailable ? "A auditoria ainda não está disponível nesta versão." : `Não foi possível ler a auditoria. ${p.cause}`
    )}</span></div></section>`;
  }
  const events = s.audit.value.filter((e) => e.kind === "blocked" || e.kind === "unconfirmed" || e.kind === "commit_allowed_emergency").slice(0, MAX_EVENTS);
  if (!events.length) {
    return `<section class="page-section" aria-labelledby="home-events-title">${head}<div class="surface home-note">${icon("shieldCheck")}<span>Nenhum commit bloqueado ou sem confirmação recentemente.</span></div></section>`;
  }
  return `<section class="page-section" aria-labelledby="home-events-title">${head}<ul class="surface int-audit-list">${events.map(auditRow).join("")}</ul></section>`;
}

function renderBody(s: State, expanded: string | null): string {
  const accounts = s.accounts.status === "ok" ? s.accounts.value : [];
  const err = [s.accounts, s.bindings].find((x) => x.status === "error");
  if (err && err.status === "error") {
    return `${renderStats(s)}${errorView({
      title: "Não foi possível carregar o Painel",
      error: err.error,
      actions: [{ label: "Tentar de novo", action: "reload", icon: "refresh", primary: true }],
    })}`;
  }
  const onboarding = s.accounts.status === "ok" && s.bindings.status === "ok" ? renderOnboarding(accounts, s.bindings.value) : "";
  return `
    ${renderStats(s)}
    ${renderAttention(s)}
    ${onboarding}
    ${renderRecent(s, accounts, expanded)}
    <div class="home-grid">
      <div class="home-col">${renderCurrent(s, accounts)}${renderToSend(s, accounts)}</div>
      <div class="home-col">${renderEvents(s)}</div>
    </div>`;
}

export const mount: MountView = (root, ctx) => {
  root.innerHTML = `
    <div class="page-head">
      <div class="heading-group"><h1>Painel</h1><p>Acompanhe as contas e as identidades dos seus projetos.</p></div>
      <button type="button" class="icon-button" id="overview-refresh" aria-label="Atualizar o Painel" title="Atualizar">${icon("refresh")}</button>
    </div>
    <div class="home" id="overview-body"></div>`;

  const body = root.querySelector<HTMLElement>("#overview-body")!;
  const refreshBtn = root.querySelector<HTMLButtonElement>("#overview-refresh")!;

  let disposed = false;
  let seq = 0;
  let expanded: string | null = null;
  const fresh = (): State => ({
    oauth: { status: "loading" },
    accounts: { status: "loading" },
    bindings: { status: "loading" },
    integration: { status: "loading" },
    repos: { status: "loading" },
    diagnostics: { status: "loading" },
    audit: { status: "loading" },
    current: { status: "loading" },
    emergency: { status: "loading" },
  });
  let state: State = fresh();

  const draw = (): void => {
    if (disposed) return;
    const active = document.activeElement as HTMLElement | null;
    const hadFocus = !!active && body.contains(active);
    const key = active?.dataset?.repoId
      ? `[data-repo-id="${CSS.escape(active.dataset.repoId)}"]`
      : active?.dataset?.nav
        ? `[data-nav="${CSS.escape(active.dataset.nav)}"]`
        : active?.dataset?.action
          ? `[data-action="${CSS.escape(active.dataset.action)}"]`
          : null;
    body.innerHTML = renderBody(state, expanded);
    if (hadFocus && key) body.querySelector<HTMLElement>(key)?.focus({ preventScroll: true });
  };

  function settle<K extends keyof State>(key: K, p: Promise<State[K] extends Loadable<infer T> ? T : never>, mine: number): void {
    void p.then(
      (value) => {
        if (disposed || mine !== seq) return;
        state = { ...state, [key]: { status: "ok", value } };
        draw();
      },
      (error) => {
        if (disposed || mine !== seq) return;
        state = { ...state, [key]: { status: "error", error } };
        draw();
      }
    );
  }

  /** `silent`: mantém o conteúdo atual até chegarem dados novos (sem saltos de layout). */
  async function load(silent: boolean): Promise<void> {
    const mine = ++seq;
    if (!silent) {
      const keep = { current: state.current, emergency: state.emergency };
      state = { ...fresh(), ...keep };
      draw();
    }
    settle("repos", reposApi.list(), mine);
    settle("integration", integrationApi.status(), mine);
    settle("audit", systemApi.audit(50), mine);
    settle(
      "diagnostics",
      systemApi.diagnostics({ probeTokens: false }).then((d) => {
        announceDiagnostics(d);
        return d;
      }),
      mine
    );

    const [oauth, accounts, bindings] = await Promise.allSettled([api.oauthStatus(), api.accounts(), api.bindings()]);
    if (disposed || mine !== seq) return;
    state = {
      ...state,
      oauth: oauth.status === "fulfilled" ? { status: "ok", value: oauth.value.ok } : { status: "error", error: oauth.reason },
      accounts: accounts.status === "fulfilled" ? { status: "ok", value: accounts.value } : { status: "error", error: accounts.reason },
      bindings: bindings.status === "fulfilled" ? { status: "ok", value: bindings.value } : { status: "error", error: bindings.reason },
    };
    draw();
  }

  async function fixAll(btn: HTMLButtonElement): Promise<void> {
    const targets = state.repos.status === "ok" ? state.repos.value.filter((r) => r.exists && r.identity.state === "different") : [];
    if (!targets.length) return;
    let fixed = 0;
    const failed: string[] = [];
    await withBusy(btn, "Corrigindo…", async () => {
      for (const r of targets) {
        try {
          await reposApi.fixIdentity(r.id);
          fixed++;
        } catch (e) {
          failed.push(`${r.github_repo ?? shortenPath(r.root_path)}: ${errMessage(e)}`);
        }
      }
    });
    if (disposed) return;
    if (fixed > 0) showToast(fixed === 1 ? "Identidade corrigida em 1 repositório." : `Identidade corrigida em ${fixed} repositórios.`, "ok");
    if (failed.length) showToast(`Não foi possível corrigir:\n${failed.join("\n")}`);
    ctx.notifyDataChanged();
    await load(true);
  }

  async function setPin(btn: HTMLButtonElement, id: string | null): Promise<void> {
    btn.disabled = true;
    try {
      await pinRepo(id);
      showToast(id ? "Repositório fixado como atual." : "Repositório desafixado.", "ok");
    } catch (e) {
      showToast(errMessage(e));
    } finally {
      if (btn.isConnected) btn.disabled = false;
    }
  }

  const onClick = (ev: Event): void => {
    const target = ev.target as HTMLElement;
    const row = target.closest<HTMLElement>(".project-row[data-repo-id]");
    if (row && body.contains(row)) {
      ctx.navigate(drawerRoute(row.dataset.repoId!));
      return;
    }
    const act = target.closest<HTMLButtonElement>("[data-action]");
    if (act && body.contains(act)) {
      switch (act.dataset.action) {
        case "fix-all":
          void fixAll(act);
          break;
        case "map-folder":
          void startFolderMap(ctx, act);
          break;
        case "welcome":
          document.dispatchEvent(new CustomEvent(WELCOME_REQUEST_EVENT));
          break;
        case "reload":
          void load(false);
          break;
        case "pin":
          void setPin(act, act.dataset.id ?? null);
          break;
        case "unpin":
          void setPin(act, null);
          break;
      }
      return;
    }
    const el = target.closest<HTMLElement>("[data-nav]");
    if (el && body.contains(el)) ctx.navigate(el.dataset.nav as Route);
  };
  body.addEventListener("click", onClick);

  refreshBtn.addEventListener("click", () => {
    void withBusy(refreshBtn, "", () => load(false)).catch(() => undefined);
  });

  const onData = (): void => void load(true);
  document.addEventListener(DATA_CHANGED_EVENT, onData);

  const onDiag = (e: Event): void => {
    const d = (e as CustomEvent<Diagnostics>).detail;
    if (!d) return;
    state = { ...state, diagnostics: { status: "ok", value: d } };
  };
  document.addEventListener(DIAGNOSTICS_UPDATED_EVENT, onDiag);

  const onDrawer = (e: Event): void => {
    expanded = (e as CustomEvent<DrawerEventDetail>).detail.id;
    body.querySelectorAll<HTMLElement>(".project-row").forEach((r) => r.setAttribute("aria-expanded", String(r.dataset.repoId === expanded)));
  };
  document.addEventListener(DRAWER_EVENT, onDrawer);

  const offSystem = onSystemState((snap) => {
    if (snap.current === state.current && snap.emergency === state.emergency) return;
    state = { ...state, current: snap.current, emergency: snap.emergency };
    draw();
  });

  let unlistenIntegration: (() => void) | null = null;
  void onIntegrationChanged((value) => {
    state = { ...state, integration: { status: "ok", value } };
    draw();
  }).then((un) => {
    if (disposed) un();
    else unlistenIntegration = un;
  });

  draw();
  void load(false);

  return () => {
    disposed = true;
    document.removeEventListener(DATA_CHANGED_EVENT, onData);
    document.removeEventListener(DIAGNOSTICS_UPDATED_EVENT, onDiag);
    document.removeEventListener(DRAWER_EVENT, onDrawer);
    body.removeEventListener("click", onClick);
    offSystem();
    unlistenIntegration?.();
  };
};
