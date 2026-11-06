import "./overview.css";
import { api, type AccountRow, type BindingRow } from "../lib/api";
import { esc, withBusy, shortenPath } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileAvatar, profileChip, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView, type ViewId } from "../lib/view";

const MAX_REPOS = 6;

type Loadable<T> = { status: "loading" } | { status: "ok"; value: T } | { status: "error"; message: string };

type State = {
  oauth: Loadable<boolean>;
  accounts: Loadable<AccountRow[]>;
  bindings: Loadable<BindingRow[]>;
  daemon: Loadable<boolean>;
};

const escAttr = esc;

function repoName(b: BindingRow): string {
  if (b.github_owner && b.github_repo) return `${b.github_owner}/${b.github_repo}`;
  const parts = b.root_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? b.root_path;
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function badge(kind: "success" | "warning" | "error" | "neutral", text: string, sym?: "check" | "alert" | "x"): string {
  const cls = kind === "neutral" ? "badge" : `badge ${kind}`;
  const glyph = kind === "neutral" ? `<span class="spinner overview-badge-spinner" aria-hidden="true"></span>` : icon(sym ?? "check", 12);
  return `<span class="${cls}">${glyph}${esc(text)}</span>`;
}

function tile(view: ViewId, ic: Parameters<typeof icon>[0], label: string, detail: string, badgeHtml: string): string {
  return `
    <button type="button" class="overview-tile" data-nav="${view}">
      <span class="overview-tile-icon">${icon(ic, 18)}</span>
      <span class="overview-tile-text">
        <span class="overview-tile-label">${esc(label)}</span>
        <span class="overview-tile-detail muted small">${esc(detail)}</span>
      </span>
      ${badgeHtml}
    </button>`;
}

function renderTiles(s: State): string {
  // OAuth
  let oauth: string;
  if (s.oauth.status === "loading") {
    oauth = tile("settings", "key", "OAuth", "A verificar…", badge("neutral", "A verificar"));
  } else if (s.oauth.status === "ok" && s.oauth.value) {
    oauth = tile("settings", "key", "OAuth", "Cliente GitHub configurado", badge("success", "Pronto", "check"));
  } else {
    oauth = tile("settings", "key", "OAuth", "Falta configurar o cliente", badge("error", "Configurar", "x"));
  }

  // Contas
  let accounts: string;
  if (s.accounts.status === "loading") {
    accounts = tile("accounts", "user", "Contas", "A carregar…", badge("neutral", "A verificar"));
  } else if (s.accounts.status === "error") {
    accounts = tile("accounts", "user", "Contas", "Não foi possível ler", badge("error", "Erro", "x"));
  } else if (s.accounts.value.length === 0) {
    accounts = tile("accounts", "user", "Contas", "Nenhuma conta ligada", badge("warning", "Nenhuma", "alert"));
  } else {
    const n = s.accounts.value.length;
    accounts = tile("accounts", "user", "Contas", plural(n, "conta ligada", "contas ligadas"), badge("success", String(n), "check"));
  }

  // gitpassd
  let daemon: string;
  if (s.daemon.status === "loading") {
    daemon = tile("settings", "activity", "gitpassd", "A verificar…", badge("neutral", "A verificar"));
  } else if (s.daemon.status === "ok") {
    daemon = tile("settings", "activity", "gitpassd", "Serviço em execução", badge("success", "Ativo", "check"));
  } else {
    daemon = tile("settings", "activity", "gitpassd", "Só necessário para hooks Git", badge("warning", "Parado", "alert"));
  }

  return `<div class="overview-tiles">${oauth}${accounts}${daemon}</div>`;
}

function step(n: string, eyebrow: string, title: string, desc: string, done: boolean, next: boolean, view: ViewId): string {
  const marker = done ? icon("check", 16) : `<span aria-hidden="true">${esc(n)}</span>`;
  const state = done ? `<span class="badge success">${icon("check", 12)}Concluído</span>` : `<span class="overview-step-go">${next ? "Começar" : "Abrir"}</span>`;
  return `
    <li>
      <button type="button" class="overview-step${done ? " done" : ""}${next ? " next" : ""}" data-nav="${view}">
        <span class="overview-step-marker${done ? " done" : ""}">${marker}</span>
        <span class="overview-step-text">
          <span class="overview-eyebrow">${esc(eyebrow)}</span>
          <span class="overview-step-title">${esc(title)}</span>
          <span class="muted small">${esc(desc)}</span>
        </span>
        ${state}
      </button>
    </li>`;
}

function renderOnboarding(accounts: AccountRow[], bindings: BindingRow[]): string {
  if (accounts.length > 0 && bindings.length > 0) return "";
  const hasAcc = accounts.length > 0;
  const hasBind = bindings.length > 0;
  return `
    <section class="panel overview-onboarding" aria-labelledby="overview-onboarding-title">
      <h2 class="panel-title" id="overview-onboarding-title">Primeiros passos</h2>
      <p class="muted small overview-onboarding-sub">Duas etapas até o Git usar sempre a identidade certa.</p>
      <ol class="list overview-steps">
        ${step("1", "01 / Conta", "Ligar conta GitHub", "Autorize uma conta para criar um perfil.", hasAcc, !hasAcc, "accounts")}
        ${step("2", "02 / Repositório", "Vincular um repositório", "Escolha uma pasta e associe-a a um perfil.", hasBind, hasAcc && !hasBind, "repos")}
      </ol>
    </section>`;
}

function renderRepos(accounts: AccountRow[], bindings: BindingRow[]): string {
  const byId = new Map(accounts.map((a) => [a.id, a]));
  const shown = bindings.slice(0, MAX_REPOS);
  const more = bindings.length > MAX_REPOS;
  const header = `
    <div class="overview-section-head">
      <h2 class="section-label overview-section-title">Repositórios${bindings.length ? ` · ${bindings.length}` : ""}</h2>
      ${more ? `<button type="button" class="btn ghost" data-nav="repos">Ver todos (${bindings.length})</button>` : ""}
    </div>`;

  if (bindings.length === 0) {
    return `
      <section class="overview-section" aria-label="Repositórios">
        ${header}
        <div class="panel">
          <div class="empty-state">
            <span class="empty-icon">${icon("repo", 24)}</span>
            <h3>Nenhum repositório vinculado</h3>
            <p>Vincule uma pasta com um repositório GitHub para que o commit use o perfil certo.</p>
            <button type="button" class="btn primary" data-nav="repos">${icon("plus", 14)}Vincular repositório</button>
          </div>
        </div>
      </section>`;
  }

  const rows = shown
    .map((b) => {
      const acc = byId.get(b.account_id);
      const chip = acc
        ? profileChip(acc)
        : `<span class="badge">${esc(b.account_login ? `@${b.account_login}` : "Conta removida")}</span>`;
      const path = shortenPath(b.root_path);
      return `
        <li>
          <button type="button" class="list-row overview-repo-row" data-nav="repos">
            <span class="overview-repo-text">
              <span class="mono overview-repo-name truncate">${esc(repoName(b))}</span>
              <span class="mono muted small truncate" title="${escAttr(b.root_path)}">${esc(path)}</span>
            </span>
            <span class="overview-repo-chip">${chip}</span>
          </button>
        </li>`;
    })
    .join("");

  return `
    <section class="overview-section" aria-label="Repositórios">
      ${header}
      <ul class="list">${rows}</ul>
    </section>`;
}

function renderProfiles(accounts: AccountRow[], bindings: BindingRow[]): string {
  const header = `<div class="overview-section-head"><h2 class="section-label overview-section-title">Perfis</h2></div>`;
  if (accounts.length === 0) {
    return `
      <section class="overview-section" aria-label="Perfis">
        ${header}
        <div class="panel overview-profiles-empty">
          <p class="muted small">Ainda não há perfis. Ligue uma conta GitHub para criar o primeiro.</p>
          <button type="button" class="btn" data-nav="accounts">${icon("plus", 14)}Ligar conta</button>
        </div>
      </section>`;
  }
  const counts = new Map<string, number>();
  for (const b of bindings) counts.set(b.account_id, (counts.get(b.account_id) ?? 0) + 1);
  const cards = accounts
    .map((a) => {
      const n = counts.get(a.id) ?? 0;
      const name = profileName(a);
      return `
        <li>
          <button type="button" class="overview-profile" data-nav="accounts">
            ${profileAvatar(a, 40)}
            <span class="overview-profile-text">
              <span class="overview-profile-name truncate">${esc(name)}</span>
              <span class="muted small truncate">${a.login ? `@${esc(a.login)}` : "Sem login"}</span>
            </span>
            <span class="overview-profile-count" title="Repositórios vinculados">
              <span class="overview-profile-num">${n}</span>
              <span class="muted small">${n === 1 ? "repo" : "repos"}</span>
            </span>
          </button>
        </li>`;
    })
    .join("");
  return `
    <section class="overview-section" aria-label="Perfis">
      ${header}
      <ul class="list overview-profile-grid">${cards}</ul>
    </section>`;
}

function renderSkeleton(): string {
  return `
    <div class="overview-grid" aria-busy="true">
      <div class="overview-section">
        <div class="overview-section-head"><h2 class="section-label overview-section-title">Repositórios</h2></div>
        <ul class="list">${Array.from({ length: 3 }, () => `<li><div class="skeleton overview-sk-row"></div></li>`).join("")}</ul>
      </div>
      <div class="overview-section">
        <div class="overview-section-head"><h2 class="section-label overview-section-title">Perfis</h2></div>
        <ul class="list overview-profile-grid">${Array.from({ length: 2 }, () => `<li><div class="skeleton overview-sk-profile"></div></li>`).join("")}</ul>
      </div>
    </div>`;
}

function renderBody(s: State): string {
  const tiles = renderTiles(s);
  const loading = s.accounts.status === "loading" || s.bindings.status === "loading";
  if (loading) return tiles + renderSkeleton();

  const err = [s.accounts, s.bindings].find((x) => x.status === "error");
  if (err && err.status === "error") {
    return `${tiles}
      <div class="callout error overview-error" role="alert">${icon("alert", 16)}<span>Não foi possível carregar os dados.\n${esc(err.message)}</span></div>`;
  }
  if (s.accounts.status !== "ok" || s.bindings.status !== "ok") return tiles;

  const accounts = s.accounts.value;
  const bindings = s.bindings.value;
  return `
    ${tiles}
    ${renderOnboarding(accounts, bindings)}
    <div class="overview-grid">
      ${renderRepos(accounts, bindings)}
      ${renderProfiles(accounts, bindings)}
    </div>`;
}

export const mount: MountView = (root, ctx) => {
  root.innerHTML = `
    <header class="view-header">
      <div>
        <h1 class="view-title">Painel</h1>
        <p class="view-sub">A conta certa em cada projeto.</p>
      </div>
      <div class="view-actions">
        <button type="button" class="btn ghost" id="overview-refresh">${icon("refresh", 14)}Atualizar</button>
      </div>
    </header>
    <div class="overview" id="overview-body"></div>`;

  const body = root.querySelector<HTMLElement>("#overview-body")!;
  const refreshBtn = root.querySelector<HTMLButtonElement>("#overview-refresh")!;

  let disposed = false;
  let seq = 0;
  let state: State = {
    oauth: { status: "loading" },
    accounts: { status: "loading" },
    bindings: { status: "loading" },
    daemon: { status: "loading" },
  };

  const draw = (): void => {
    if (!disposed) body.innerHTML = renderBody(state);
  };

  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  /** `silent`: mantém o conteúdo atual até chegarem dados novos (sem saltos de layout). */
  async function load(silent: boolean): Promise<void> {
    const mine = ++seq;
    if (!silent) {
      state = {
        oauth: { status: "loading" },
        accounts: { status: "loading" },
        bindings: { status: "loading" },
        daemon: { status: "loading" },
      };
      draw();
    }

    // O ping do daemon pode demorar: atualiza o tile à parte.
    const ping = api.pingDaemon().then(
      (): Loadable<boolean> => ({ status: "ok", value: true }),
      (e): Loadable<boolean> => ({ status: "error", message: msg(e) })
    );
    void ping.then((daemon) => {
      if (disposed || mine !== seq) return;
      state = { ...state, daemon };
      draw();
    });

    const [oauth, accounts, bindings] = await Promise.allSettled([api.oauthStatus(), api.accounts(), api.bindings()]);
    if (disposed || mine !== seq) return;
    state = {
      ...state,
      oauth: oauth.status === "fulfilled" ? { status: "ok", value: oauth.value.ok } : { status: "error", message: msg(oauth.reason) },
      accounts: accounts.status === "fulfilled" ? { status: "ok", value: accounts.value } : { status: "error", message: msg(accounts.reason) },
      bindings: bindings.status === "fulfilled" ? { status: "ok", value: bindings.value } : { status: "error", message: msg(bindings.reason) },
    };
    draw();
  }

  const onClick = (ev: Event): void => {
    const el = (ev.target as HTMLElement).closest<HTMLElement>("[data-nav]");
    if (el && body.contains(el)) ctx.navigate(el.dataset.nav as ViewId);
  };
  body.addEventListener("click", onClick);

  refreshBtn.addEventListener("click", () => {
    void withBusy(refreshBtn, "A atualizar…", () => load(false)).catch(() => undefined);
  });

  const onData = (): void => void load(true);
  document.addEventListener(DATA_CHANGED_EVENT, onData);

  draw();
  void load(false);

  return () => {
    disposed = true;
    document.removeEventListener(DATA_CHANGED_EVENT, onData);
    body.removeEventListener("click", onClick);
  };
};
