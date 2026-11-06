import "./repos.css";
import { api, type AccountRow } from "../lib/api";
import {
  reposApi,
  type BindOutcome,
  type DiscoveredRepo,
  type RepoRow,
  type SuggestionReason,
} from "../lib/api-repos";
import { errMessage, esc, shortenPath, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileChip, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView } from "../lib/view";

/** Seleção mantida em memória entre recarregamentos e mudanças de ecrã. */
let rememberedSelection: string | null = null;

type DiscoveryStatus = "idle" | "scanning" | "done" | "error";

/** Resultados de «Encontrados neste Mac», mantidos durante a sessão (a procura corre uma vez automaticamente). */
const discovery = {
  status: "idle" as DiscoveryStatus,
  items: [] as DiscoveredRepo[],
  error: "",
  truncated: false,
  extraRoots: [] as string[],
  /** Conta escolhida por item (caminho → id da conta). */
  choice: new Map<string, string>(),
  run: null as Promise<void> | null,
};

/** Re-renderiza a secção de descoberta da vista montada (quando a procura termina fora do ciclo da vista). */
let onDiscoveryUpdate: (() => void) | null = null;

type ProfileRef = Pick<AccountRow, "id" | "login" | "label">;

const REASON_TEXT: Record<SuggestionReason, string> = {
  owner_login: "conta dona do repositório",
  same_owner_binding: "já usada noutros repositórios deste dono",
  only_account: "única conta ligada",
  push_access: "única conta com permissão de push",
};

function repoName(b: Pick<RepoRow, "github_owner" | "github_repo" | "root_path">): string {
  if (b.github_owner && b.github_repo) return `${b.github_owner}/${b.github_repo}`;
  const parts = b.root_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? b.root_path;
}

const shortPath = shortenPath;

function parseCommit(line: string): { hash: string | null; message: string } {
  const m = /^([0-9a-f]{7,40})\b\s*(.*)$/i.exec(line.trim());
  return m ? { hash: m[1], message: m[2] } : { hash: null, message: line };
}

function identityText(name: string | null, email: string | null): string {
  if (!name && !email) return "não definida";
  return `${name ?? "(sem nome)"} <${email ?? "sem email"}>`;
}

type RowStatus = { kind: "success" | "warning" | "error"; symbol: string; label: string; fixable: boolean };

function rowStatus(b: RepoRow): RowStatus {
  if (!b.exists) return { kind: "error", symbol: "✕", label: "Pasta não encontrada", fixable: false };
  if (b.account_missing) return { kind: "error", symbol: "✕", label: "Conta removida", fixable: false };
  switch (b.identity.state) {
    case "correct":
      return { kind: "success", symbol: "✓", label: "Identidade correta", fixable: false };
    case "different":
      return { kind: "warning", symbol: "⚠", label: "Identidade diferente", fixable: true };
    default:
      return { kind: "warning", symbol: "⚠", label: "Identidade não definida", fixable: true };
  }
}

function statusBadge(s: RowStatus): string {
  return `<span class="badge ${s.kind}"><span aria-hidden="true">${s.symbol}</span> ${esc(s.label)}</span>`;
}

type ModalOptions<T> = {
  title: string;
  body: string;
  confirmLabel: string;
  /** Ação destrutiva: o foco inicial fica em «Cancelar». */
  danger?: boolean;
  /** Lê o resultado do formulário ao confirmar; devolver `undefined` mantém o modal aberto. */
  read: (dialog: HTMLElement) => T | undefined;
};

/** Modal de confirmação do kit; resolve com o valor de `read` ou `null` se for cancelado. */
function openModal<T>(opts: ModalOptions<T>): Promise<T | null> {
  const opener = document.activeElement as HTMLElement | null;
  const backdrop = document.createElement("div");
  backdrop.className = "modal-backdrop repos-modal-backdrop";
  backdrop.innerHTML = `
    <div class="modal repos-modal" role="dialog" aria-modal="true" aria-labelledby="repos-modal-title">
      <header class="repos-modal-head">
        <h2 class="repos-modal-title" id="repos-modal-title">${esc(opts.title)}</h2>
        <button type="button" class="btn ghost icon-only" data-modal="cancel" aria-label="Fechar">${icon("x")}</button>
      </header>
      <div class="repos-modal-body">${opts.body}</div>
      <footer class="repos-modal-foot">
        <button type="button" class="btn ghost" data-modal="cancel">Cancelar</button>
        <button type="button" class="btn primary${opts.danger ? " repos-danger" : ""}" data-modal="confirm">${esc(opts.confirmLabel)}</button>
      </footer>
    </div>`;
  document.body.appendChild(backdrop);
  const dialog = backdrop.querySelector<HTMLElement>(".modal")!;

  return new Promise<T | null>((resolve) => {
    const done = (value: T | null): void => {
      document.removeEventListener("keydown", onKey, true);
      backdrop.remove();
      opener?.focus();
      resolve(value);
    };
    const focusables = (): HTMLElement[] =>
      Array.from(dialog.querySelectorAll<HTMLElement>("button, input, select, [tabindex]:not([tabindex='-1'])")).filter(
        (el) => !(el as HTMLButtonElement).disabled
      );
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        done(null);
      } else if (e.key === "Tab") {
        const els = focusables();
        const first = els[0];
        const last = els[els.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last?.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first?.focus();
        }
      }
    };
    document.addEventListener("keydown", onKey, true);
    backdrop.addEventListener("pointerdown", (e) => {
      if (e.target === backdrop) done(null);
    });
    dialog.addEventListener("click", (e) => {
      const action = (e.target as HTMLElement).closest<HTMLElement>("[data-modal]")?.dataset.modal;
      if (action === "cancel") done(null);
      if (action === "confirm") {
        const v = opts.read(dialog);
        if (v !== undefined) done(v);
      }
    });
    const initial = opts.danger
      ? dialog.querySelector<HTMLElement>(".repos-modal-foot [data-modal='cancel']")
      : (dialog.querySelector<HTMLElement>("input:checked") ??
        dialog.querySelector<HTMLElement>("input") ??
        dialog.querySelector<HTMLElement>("[data-modal='confirm']"));
    initial?.focus();
  });
}

export const mount: MountView = (root, ctx) => {
  let accounts: AccountRow[] = [];
  let repos: RepoRow[] = [];
  let query = "";
  let loadToken = 0;
  let logToken = 0;
  let disposed = false;
  let closeMenu: (() => void) | null = null;

  const profileFor = (b: Pick<RepoRow, "account_id" | "account_login" | "account_label">): ProfileRef => {
    const acc = accounts.find((a) => a.id === b.account_id);
    if (acc) return acc;
    return { id: b.account_id, login: b.account_login, label: b.account_label };
  };

  const accountById = (id: string | null): AccountRow | undefined => accounts.find((a) => a.id === id);
  const who = (id: string): string => {
    const a = accountById(id);
    return a ? (a.login ? `@${a.login}` : profileName(a)) : "outra conta";
  };

  const filtered = (): RepoRow[] => {
    const q = query.trim().toLowerCase();
    if (!q) return repos;
    return repos.filter((b) =>
      [repoName(b), b.root_path, profileName(profileFor(b)), b.account_login ?? "", b.branch ?? ""].some((s) =>
        s.toLowerCase().includes(q)
      )
    );
  };

  const bindButton = (extra = ""): string =>
    accounts.length === 0
      ? `<button type="button" class="btn primary ${extra}" data-action="go-accounts">${icon("user")}<span>Ligar uma conta primeiro</span></button>`
      : `<button type="button" class="btn primary ${extra}" data-action="bind">${icon("plus")}<span>Vincular repositório</span></button>`;

  const subtitle = (): string => {
    const n = repos.length;
    if (n === 0) return "Nenhum repositório local vinculado";
    const issues = repos.filter((r) => rowStatus(r).kind !== "success").length;
    const base = n === 1 ? "1 repositório local vinculado" : `${n} repositórios locais vinculados`;
    return issues === 0 ? base : `${base} · ${issues === 1 ? "1 precisa de atenção" : `${issues} precisam de atenção`}`;
  };

  function renderLoading(): void {
    root.innerHTML = `
      <header class="view-header">
        <div>
          <h1 class="view-title">Repositórios</h1>
          <p class="view-sub">A carregar…</p>
        </div>
      </header>
      <div class="repos-layout" aria-busy="true">
        <section class="repos-list-pane">
          ${Array.from({ length: 4 }, () => `<div class="skeleton repos-skeleton-row"></div>`).join("")}
        </section>
        <section class="panel repos-detail"><div class="skeleton repos-skeleton-title"></div></section>
      </div>`;
  }

  function renderError(message: string): void {
    root.innerHTML = `
      <header class="view-header">
        <div><h1 class="view-title">Repositórios</h1></div>
        <div class="view-actions">
          <button type="button" class="btn" data-action="retry">${icon("refresh")}<span>Tentar novamente</span></button>
        </div>
      </header>
      <div class="callout error" role="alert">${icon("alert")}<span>Não foi possível carregar os repositórios: ${esc(message)}</span></div>`;
    root.querySelector<HTMLButtonElement>('[data-action="retry"]')?.addEventListener("click", () => void load());
  }

  function render(): void {
    closeMenu?.();
    const header = `
      <header class="view-header">
        <div>
          <h1 class="view-title">Repositórios</h1>
          <p class="view-sub">${esc(subtitle())}</p>
        </div>
        <div class="view-actions repos-actions">${bindButton()}</div>
      </header>`;

    const discoverySection = accounts.length > 0 ? `<section class="panel repos-discover" aria-labelledby="repos-discover-title"></section>` : "";

    if (repos.length === 0) {
      root.innerHTML = `${header}
        <section class="panel">
          <div class="empty-state">
            <div class="empty-icon">${icon(accounts.length === 0 ? "user" : "repo", 26)}</div>
            <h3>${accounts.length === 0 ? "Ligue primeiro uma conta GitHub" : "Nenhum repositório vinculado"}</h3>
            <p>${
              accounts.length === 0
                ? "Depois de ligar uma conta, o GitPass encontra os seus clones e passa a usar a conta certa em cada um."
                : "Escolha a pasta de um clone e o GitPass escolhe a conta certa. A partir daí, commits e pushes usam essa conta automaticamente."
            }</p>
            <div class="repos-actions">${bindButton("lg")}</div>
          </div>
        </section>
        ${discoverySection}`;
      wireHeaderButtons();
      renderDiscovery();
      return;
    }

    if (!rememberedSelection || !repos.some((b) => b.id === rememberedSelection)) {
      rememberedSelection = repos[0].id;
    }

    root.innerHTML = `${header}
      <div class="repos-layout">
        <section class="repos-list-pane" aria-label="Lista de repositórios">
          <label class="repos-search">
            <span class="sr-only">Procurar repositórios</span>
            ${icon("search")}
            <input type="search" class="input" placeholder="Procurar por repositório, pasta, branch ou perfil" value="${esc(query)}" autocomplete="off" spellcheck="false" />
          </label>
          <ul class="list repos-list" aria-label="Repositórios vinculados"></ul>
        </section>
        <section class="panel repos-detail" aria-live="polite"></section>
      </div>
      ${discoverySection}`;

    const input = root.querySelector<HTMLInputElement>(".repos-search input")!;
    input.addEventListener("input", () => {
      query = input.value;
      renderList();
    });
    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        focusSelected();
      }
    });

    const list = root.querySelector<HTMLUListElement>(".repos-list")!;
    list.addEventListener("click", (e) => {
      const target = e.target as HTMLElement;
      const fix = target.closest<HTMLButtonElement>('[data-action="fix-row"]');
      if (fix?.dataset.id) {
        void fixIdentity(fix, fix.dataset.id);
        return;
      }
      const row = target.closest<HTMLButtonElement>(".repos-row-select");
      if (row?.dataset.id) select(row.dataset.id, true);
    });
    list.addEventListener("keydown", onListKey);

    wireHeaderButtons();
    renderList();
    renderDetail();
    renderDiscovery();
  }

  function renderList(): void {
    const list = root.querySelector<HTMLUListElement>(".repos-list");
    if (!list) return;
    const items = filtered();
    if (items.length === 0) {
      list.innerHTML = `<li class="repos-no-results muted small">${icon("search")} Nenhum resultado para «${esc(query.trim())}».</li>`;
      return;
    }
    const selectedVisible = items.some((b) => b.id === rememberedSelection);
    list.innerHTML = items
      .map((b, i) => {
        const sel = b.id === rememberedSelection;
        const focusable = sel || (!selectedVisible && i === 0);
        const st = rowStatus(b);
        return `<li class="list-row repos-row${sel ? " selected" : ""}" data-id="${esc(b.id)}">
          <button type="button" class="repos-row-select" data-id="${esc(b.id)}" aria-current="${sel}" tabindex="${focusable ? 0 : -1}">
            <span class="repos-row-main">
              <span class="mono repos-row-name truncate">${esc(repoName(b))}</span>
              <span class="mono muted small repos-row-path truncate" title="${esc(b.root_path)}">${esc(shortPath(b.root_path))}</span>
              <span class="repos-row-status">${statusBadge(st)}</span>
            </span>
            ${profileChip(profileFor(b))}
          </button>
          ${
            st.fixable
              ? `<button type="button" class="btn repos-row-fix" data-action="fix-row" data-id="${esc(b.id)}" aria-label="Corrigir identidade de ${esc(repoName(b))}">${icon("check", 14)}<span>Corrigir</span></button>`
              : ""
          }
        </li>`;
      })
      .join("");
  }

  function focusSelected(): void {
    root.querySelector<HTMLButtonElement>('.repos-row-select[tabindex="0"]')?.focus();
  }

  function onListKey(e: KeyboardEvent): void {
    if (!(e.target as HTMLElement).classList.contains("repos-row-select")) return;
    const items = filtered();
    if (items.length === 0) return;
    const cur = (e.target as HTMLElement).dataset.id;
    let idx = items.findIndex((b) => b.id === (cur ?? rememberedSelection));
    switch (e.key) {
      case "ArrowDown":
        idx = Math.min(items.length - 1, idx + 1);
        break;
      case "ArrowUp":
        if (idx <= 0) {
          e.preventDefault();
          root.querySelector<HTMLInputElement>(".repos-search input")?.focus();
          return;
        }
        idx -= 1;
        break;
      case "Home":
        idx = 0;
        break;
      case "End":
        idx = items.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    select(items[Math.max(0, idx)].id, true);
  }

  function select(id: string, focus: boolean): void {
    const changed = id !== rememberedSelection;
    rememberedSelection = id;
    root.querySelectorAll<HTMLLIElement>(".repos-row").forEach((li) => {
      const sel = li.dataset.id === id;
      li.classList.toggle("selected", sel);
      const btn = li.querySelector<HTMLButtonElement>(".repos-row-select");
      if (!btn) return;
      btn.setAttribute("aria-current", String(sel));
      btn.tabIndex = sel ? 0 : -1;
      if (sel && focus) {
        btn.focus();
        li.scrollIntoView({ block: "nearest" });
      }
    });
    if (changed) renderDetail();
  }

  function identityCallout(b: RepoRow): string {
    const id = b.identity;
    if (!b.exists) {
      return `<div class="callout error" role="alert">${icon("alert")}<span>A pasta já não existe neste Mac. Se a moveu, desvincule-a e volte a vinculá-la no novo local.</span></div>`;
    }
    if (b.account_missing) {
      return `<div class="callout error" role="alert">${icon("alert")}<span>A conta deste vínculo foi removida. Escolha «Mudar conta» para continuar a usar o GitPass neste repositório.</span></div>`;
    }
    const expected = esc(identityText(id.expected_name, id.expected_email));
    switch (id.state) {
      case "correct":
        return `<div class="callout success">${icon("check")}<span><strong>Identidade correta.</strong> Os commits usam <span class="mono">${esc(
          identityText(id.local_name, id.local_email)
        )}</span>.${
          id.expected_email ? "" : " A conta não tem email de commit definido; escolha um em Contas."
        }</span></div>`;
      case "different":
        return `<div class="callout warning repos-identity-callout">${icon("alert")}<div>
          <p><strong>Identidade diferente da conta.</strong> Os próximos commits não vão ficar associados a ${esc(who(b.account_id))}.</p>
          <dl class="repos-identity-diff">
            <dt>Aplicada</dt><dd class="mono">${esc(identityText(id.local_name, id.local_email))}</dd>
            <dt>Da conta</dt><dd class="mono">${expected}</dd>
          </dl>
          <button type="button" class="btn" data-action="fix">${icon("check")}<span>Corrigir identidade</span></button>
        </div></div>`;
      default:
        return `<div class="callout warning repos-identity-callout">${icon("alert")}<div>
          <p><strong>Identidade não definida.</strong> Os commits usam a identidade global do Git em vez de <span class="mono">${expected}</span>.</p>
          <button type="button" class="btn" data-action="fix">${icon("check")}<span>Aplicar identidade da conta</span></button>
        </div></div>`;
    }
  }

  function renderDetail(): void {
    closeMenu?.();
    const panel = root.querySelector<HTMLElement>(".repos-detail");
    if (!panel) return;
    const b = repos.find((x) => x.id === rememberedSelection);
    if (!b) {
      panel.innerHTML = `<p class="muted small">Selecione um repositório.</p>`;
      return;
    }
    const head = b.branch
      ? `<span class="mono">${esc(b.branch)}</span>`
      : b.detached_at
        ? `<span class="mono">HEAD desanexado em ${esc(b.detached_at)}</span>`
        : `<span class="muted">—</span>`;
    panel.innerHTML = `
      <div class="repos-detail-head">
        <div class="repos-detail-title">
          <span class="repos-detail-icon">${icon("repo", 20)}</span>
          <h2 class="panel-title mono truncate" title="${esc(repoName(b))}">${esc(repoName(b))}</h2>
        </div>
        ${profileChip(profileFor(b), { size: "md" })}
      </div>
      <div class="repos-detail-callouts">
        ${identityCallout(b)}
        ${
          b.remote_has_credentials
            ? `<div class="callout error" role="alert">${icon("lock")}<span>O <span class="mono">origin</span> deste repositório tem uma palavra-passe ou token escrito no URL. Substitua-o por <span class="mono">${esc(
                b.remote_url ?? ""
              )}</span> — o GitPass fornece as credenciais automaticamente.</span></div>`
            : ""
        }
      </div>
      <div class="repos-detail-actions" role="group" aria-label="Ações do repositório">
        <span class="repos-menu-host">
          <button type="button" class="btn" data-action="change-account" aria-haspopup="menu" aria-expanded="false"${
            accounts.length === 0 ? " disabled" : ""
          }>${icon("user")}<span>Mudar conta</span></button>
        </span>
        <button type="button" class="btn" data-action="finder"${b.exists ? "" : " disabled"}>${icon("folder")}<span>Mostrar no Finder</span></button>
        <button type="button" class="btn" data-action="terminal"${b.exists ? "" : " disabled"}>${icon("monitor")}<span>Abrir no Terminal</span></button>
        <button type="button" class="btn ghost repos-danger-text" data-action="unbind">${icon("x")}<span>Desvincular</span></button>
      </div>
      <dl class="repos-meta">
        <div class="repos-meta-item">
          <dt class="field-label">Pasta local</dt>
          <dd class="repos-path-row">
            <code class="repos-path mono">${esc(b.root_path)}</code>
            <button type="button" class="btn ghost icon-only" data-action="copy-path" aria-label="Copiar caminho" title="Copiar caminho">${icon("copy")}</button>
          </dd>
        </div>
        <div class="repos-meta-grid">
          <div class="repos-meta-item">
            <dt class="field-label">GitHub</dt>
            <dd>${
              b.remote_url
                ? `<a class="repos-gh-link mono" href="${esc(b.remote_url)}" data-action="open-gh">${esc(
                    b.remote_url.replace(/^https:\/\//, "")
                  )} ${icon("external", 14)}<span class="sr-only">(abre no navegador)</span></a>`
                : `<span class="badge warning"><span aria-hidden="true">⚠</span> Origin desconhecido</span>`
            }</dd>
          </div>
          <div class="repos-meta-item">
            <dt class="field-label">Branch atual</dt>
            <dd class="repos-branch">${icon("branch", 14)} ${head}</dd>
          </div>
          <div class="repos-meta-item">
            <dt class="field-label">user.name</dt>
            <dd class="mono repos-ident-value">${b.identity.local_name ? esc(b.identity.local_name) : `<span class="muted">não definido</span>`}</dd>
          </div>
          <div class="repos-meta-item">
            <dt class="field-label">user.email</dt>
            <dd class="mono repos-ident-value">${b.identity.local_email ? esc(b.identity.local_email) : `<span class="muted">não definido</span>`}</dd>
          </div>
        </div>
      </dl>
      <h3 class="section-label repos-log-label">${icon("commit", 14)} Histórico de commits</h3>
      <div class="repos-log"></div>`;

    panel.querySelector<HTMLButtonElement>('[data-action="fix"]')?.addEventListener("click", (e) =>
      void fixIdentity(e.currentTarget as HTMLButtonElement, b.id)
    );
    panel.querySelector<HTMLAnchorElement>('[data-action="open-gh"]')?.addEventListener("click", (e) => {
      e.preventDefault();
      if (b.remote_url) api.openExternal(b.remote_url).catch((err) => showToast(errMessage(err), "err"));
    });
    panel.querySelector<HTMLButtonElement>('[data-action="finder"]')?.addEventListener("click", () =>
      reposApi.openInFinder(b.root_path).catch((err) => showToast(errMessage(err), "err"))
    );
    panel.querySelector<HTMLButtonElement>('[data-action="terminal"]')?.addEventListener("click", () =>
      reposApi.openInTerminal(b.root_path).catch((err) => showToast(errMessage(err), "err"))
    );
    panel.querySelector<HTMLButtonElement>('[data-action="unbind"]')?.addEventListener("click", () => void unbind(b));
    const changeBtn = panel.querySelector<HTMLButtonElement>('[data-action="change-account"]')!;
    changeBtn.addEventListener("click", () => {
      if (closeMenu) {
        closeMenu();
        return;
      }
      openAccountMenu(changeBtn, "Usar neste repositório", b.account_id, (id) => void changeAccount(changeBtn, b, id));
    });

    const copyBtn = panel.querySelector<HTMLButtonElement>('[data-action="copy-path"]')!;
    copyBtn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(b.root_path);
        copyBtn.innerHTML = icon("check");
        copyBtn.setAttribute("aria-label", "Caminho copiado");
        showToast("Caminho copiado para a área de transferência.", "ok");
        window.setTimeout(() => {
          if (!copyBtn.isConnected) return;
          copyBtn.innerHTML = icon("copy");
          copyBtn.setAttribute("aria-label", "Copiar caminho");
        }, 1600);
      } catch (e) {
        showToast(`Não foi possível copiar: ${errMessage(e)}`, "err");
      }
    });

    const logBox = panel.querySelector<HTMLElement>(".repos-log")!;
    if (b.exists) void loadLog(b, logBox);
    else logBox.innerHTML = `<p class="muted small repos-log-empty">${icon("commit")} Histórico indisponível: a pasta não foi encontrada.</p>`;
  }

  async function loadLog(b: RepoRow, box: HTMLElement): Promise<void> {
    const token = ++logToken;
    box.setAttribute("aria-busy", "true");
    box.innerHTML = `<span class="sr-only">A carregar commits…</span>${Array.from(
      { length: 6 },
      (_, i) => `<div class="skeleton repos-skeleton-commit" style="width:${92 - (i % 3) * 14}%"></div>`
    ).join("")}`;
    try {
      const lines = await api.commitLog(b.root_path, 40);
      if (disposed || token !== logToken) return;
      box.removeAttribute("aria-busy");
      if (lines.length === 0) {
        box.innerHTML = `<p class="muted small repos-log-empty">${icon("commit")} Ainda não há commits neste repositório.</p>`;
        return;
      }
      box.innerHTML = `<ol class="repos-commits">${lines
        .map((line) => {
          const c = parseCommit(line);
          return `<li class="repos-commit">${
            c.hash ? `<span class="mono repos-hash" title="${esc(c.hash)}">${esc(c.hash.slice(0, 7))}</span>` : ""
          }<span class="repos-msg">${esc(c.message)}</span></li>`;
        })
        .join("")}</ol>`;
    } catch (e) {
      if (disposed || token !== logToken) return;
      box.removeAttribute("aria-busy");
      // Repositório sem commits: o git log falha em vez de devolver vazio.
      if (/does not have any commits|bad default revision/i.test(errMessage(e))) {
        box.innerHTML = `<p class="muted small repos-log-empty">${icon("commit")} Ainda não há commits neste repositório.</p>`;
        return;
      }
      box.innerHTML = `<div class="callout error" role="alert">${icon("alert")}<span>Não foi possível ler o histórico: ${esc(
        errMessage(e)
      )}</span></div>`;
    }
  }

  function wireHeaderButtons(): void {
    root.querySelectorAll<HTMLButtonElement>('[data-action="bind"]').forEach((btn) => {
      btn.addEventListener("click", () => void bindWithDialog(btn));
    });
    root.querySelectorAll<HTMLButtonElement>('[data-action="go-accounts"]').forEach((btn) => {
      btn.addEventListener("click", () => ctx.navigate("accounts"));
    });
  }

  /** Menu de perfis (vidro). `currentId` fica marcado e não é selecionável. */
  function openAccountMenu(btn: HTMLButtonElement, title: string, currentId: string | null, onPick: (id: string) => void): void {
    const host = btn.parentElement!;
    const menu = document.createElement("div");
    menu.className = "material repos-menu";
    menu.setAttribute("role", "menu");
    menu.setAttribute("aria-label", title);
    menu.innerHTML = `<p class="repos-menu-title muted small">${esc(title)}</p>${accounts
      .map((a) => {
        const cur = a.id === currentId;
        return `<button type="button" class="repos-menu-item" role="menuitemradio" aria-checked="${cur}" data-account="${esc(a.id)}"${
          cur ? ' aria-disabled="true"' : ""
        }>${profileChip(a)}${
          cur
            ? `<span class="repos-menu-current small">${icon("check", 14)} Atual</span>`
            : a.login && a.label
              ? `<span class="mono muted small truncate">@${esc(a.login)}</span>`
              : ""
        }</button>`;
      })
      .join("")}`;
    host.classList.add("repos-menu-host");
    host.appendChild(menu);
    btn.setAttribute("aria-expanded", "true");
    const items = Array.from(menu.querySelectorAll<HTMLButtonElement>(".repos-menu-item"));
    (items.find((i) => i.getAttribute("aria-checked") !== "true") ?? items[0])?.focus();

    const onDocDown = (e: PointerEvent): void => {
      if (!menu.contains(e.target as Node) && !btn.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent): void => {
      const i = items.indexOf(document.activeElement as HTMLButtonElement);
      if (e.key === "Escape") {
        e.preventDefault();
        close();
        btn.focus();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        items[(i + 1) % items.length]?.focus();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        items[(i - 1 + items.length) % items.length]?.focus();
      } else if (e.key === "Tab") {
        close();
      }
    };
    const close = (): void => {
      document.removeEventListener("pointerdown", onDocDown, true);
      menu.removeEventListener("keydown", onKey);
      menu.remove();
      btn.setAttribute("aria-expanded", "false");
      closeMenu = null;
    };
    document.addEventListener("pointerdown", onDocDown, true);
    menu.addEventListener("keydown", onKey);
    menu.addEventListener("click", (e) => {
      const item = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-account]");
      if (!item?.dataset.account || item.getAttribute("aria-checked") === "true") return;
      const id = item.dataset.account;
      close();
      btn.focus();
      onPick(id);
    });
    closeMenu = close;
  }

  function upsertRepo(row: RepoRow): void {
    const i = repos.findIndex((r) => r.id === row.id);
    if (i >= 0) repos[i] = row;
    else repos.push(row);
  }

  function afterBound(row: RepoRow, reason: SuggestionReason | null): void {
    upsertRepo(row);
    rememberedSelection = row.id;
    forgetDiscovered(row.root_path);
    const why = reason ? ` (${REASON_TEXT[reason]})` : "";
    showToast(`${repoName(row)} vinculado a ${who(row.account_id)}${why}.`, "ok");
    render();
  }

  async function bindWithDialog(btn: HTMLButtonElement): Promise<void> {
    let outcome: BindOutcome;
    try {
      outcome = await withBusy(btn, "À espera da pasta…", () => reposApi.bindDialog(null));
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
      const row = await withBusy(btn, "A vincular…", () => reposApi.bindPath(outcome.root_path, accountId));
      if (!disposed) afterBound(row, null);
    } catch (e) {
      showToast(errMessage(e) || "Não foi possível vincular o repositório.", "err");
    }
  }

  function confirmAccount(o: Extract<BindOutcome, { status: "needs_account" }>): Promise<string | null> {
    const pushText = (v: boolean | null | undefined): string =>
      v === true
        ? `<span class="badge success"><span aria-hidden="true">✓</span> Pode fazer push</span>`
        : v === false
          ? `<span class="badge warning"><span aria-hidden="true">⚠</span> Sem permissão de push</span>`
          : `<span class="badge"><span aria-hidden="true">?</span> Não confirmado</span>`;
    const preselected = o.suggested_account_id ?? accounts[0]?.id;
    const body = `
      <p class="repos-modal-lead">Qual conta deve ser usada em <span class="mono">${esc(`${o.github_owner}/${o.github_repo}`)}</span>?</p>
      <p class="muted small repos-modal-path mono" title="${esc(o.root_path)}">${esc(shortPath(o.root_path))}</p>
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
      body,
      confirmLabel: "Vincular",
      read: (d) => d.querySelector<HTMLInputElement>('input[name="repos-pick"]:checked')?.value,
    });
  }

  async function fixIdentity(btn: HTMLButtonElement, id: string): Promise<void> {
    try {
      const row = await withBusy(btn, "A corrigir…", () => reposApi.fixIdentity(id));
      if (disposed) return;
      upsertRepo(row);
      const st = rowStatus(row);
      showToast(
        st.kind === "success"
          ? `Identidade aplicada: ${identityText(row.identity.local_name, row.identity.local_email)}.`
          : "Identidade aplicada, mas a conta não tem email de commit. Escolha um em Contas.",
        st.kind === "success" ? "ok" : "err"
      );
      render();
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  }

  async function changeAccount(btn: HTMLButtonElement, b: RepoRow, accountId: string): Promise<void> {
    try {
      const row = await withBusy(btn, "A mudar…", () => reposApi.changeAccount(b.id, accountId));
      if (disposed) return;
      upsertRepo(row);
      showToast(`${repoName(row)} passa a usar ${who(accountId)}. Identidade atualizada.`, "ok");
      render();
    } catch (e) {
      showToast(errMessage(e), "err");
      void load();
    }
  }

  async function unbind(b: RepoRow): Promise<void> {
    const reset = await openModal<boolean>({
      title: `Desvincular ${repoName(b)}?`,
      body: `
        <p class="repos-modal-lead">O GitPass deixa de escolher a conta neste repositório. A pasta, os ficheiros e o histórico não são alterados.</p>
        ${
          b.exists && (b.identity.local_name || b.identity.local_email)
            ? `<label class="repos-check">
                <input type="checkbox" name="reset" />
                <span>Remover também o <span class="mono">user.name</span> e o <span class="mono">user.email</span> que o GitPass aplicou</span>
              </label>`
            : ""
        }`,
      confirmLabel: "Desvincular",
      danger: true,
      read: (d) => d.querySelector<HTMLInputElement>('input[name="reset"]')?.checked ?? false,
    });
    if (reset === null || disposed) return;
    try {
      await reposApi.unbind(b.id, reset);
      if (disposed) return;
      repos = repos.filter((r) => r.id !== b.id);
      if (rememberedSelection === b.id) rememberedSelection = null;
      showToast(`${repoName(b)} foi desvinculado.`, "ok");
      render();
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  }

  // ---------- Encontrados neste Mac ----------

  function forgetDiscovered(path: string): void {
    discovery.items = discovery.items.filter((d) => d.root_path !== path);
    discovery.choice.delete(path);
  }

  function choiceFor(d: DiscoveredRepo): string | null {
    const c = discovery.choice.get(d.root_path) ?? d.suggested_account_id;
    return c && accounts.some((a) => a.id === c) ? c : null;
  }

  function renderDiscovery(): void {
    const sec = root.querySelector<HTMLElement>(".repos-discover");
    if (!sec) return;
    const bound = new Set(repos.map((r) => r.root_path));
    discovery.items = discovery.items.filter((d) => !bound.has(d.root_path));
    const ready = discovery.items.filter((d) => choiceFor(d));
    const scanning = discovery.status === "scanning";

    let body: string;
    if (scanning && discovery.items.length === 0) {
      body = `<div class="repos-discover-list" aria-busy="true"><span class="sr-only">A procurar repositórios…</span>${Array.from(
        { length: 3 },
        () => `<div class="skeleton repos-skeleton-row"></div>`
      ).join("")}</div>`;
    } else if (discovery.status === "error") {
      body = `<div class="callout error" role="alert">${icon("alert")}<span>Não foi possível procurar repositórios: ${esc(discovery.error)}</span></div>`;
    } else if (discovery.items.length === 0) {
      body = `<p class="muted small repos-discover-empty">${icon("check")} ${
        discovery.status === "done"
          ? "Não há clones do GitHub por vincular nas pastas habituais (Developer, Projects, GitHub, Code, src, Documentos/GitHub, Secretária)."
          : "Procure clones do GitHub que já existam neste Mac."
      }</p>`;
    } else {
      body = `<ul class="list repos-discover-list">${discovery.items
        .map((d) => {
          const chosen = choiceFor(d);
          const reason =
            chosen && chosen === d.suggested_account_id && d.suggestion_reason
              ? `<span class="muted small">Sugerida: ${esc(REASON_TEXT[d.suggestion_reason])}</span>`
              : !chosen
                ? `<span class="warn small"><span aria-hidden="true">⚠</span> Escolha a conta</span>`
                : "";
          return `<li class="list-row repos-discover-row" data-path="${esc(d.root_path)}">
            <span class="repos-row-main">
              <span class="mono repos-row-name truncate">${esc(`${d.github_owner}/${d.github_repo}`)}</span>
              <span class="mono muted small repos-row-path truncate" title="${esc(d.root_path)}">${esc(shortPath(d.root_path))}</span>
            </span>
            <span class="repos-discover-pick">
              <select class="select repos-discover-select" aria-label="Conta para ${esc(`${d.github_owner}/${d.github_repo}`)}">
                ${chosen ? "" : `<option value="" selected>Escolher conta…</option>`}
                ${accounts
                  .map((a) => `<option value="${esc(a.id)}"${a.id === chosen ? " selected" : ""}>${esc(a.login ? `${profileName(a)} (@${a.login})` : profileName(a))}</option>`)
                  .join("")}
              </select>
              ${reason}
            </span>
            <button type="button" class="btn" data-action="bind-found"${chosen ? "" : " disabled"}>${icon("plus", 14)}<span>Vincular</span></button>
          </li>`;
        })
        .join("")}</ul>`;
    }

    const truncatedNote =
      discovery.truncated && discovery.status === "done"
        ? `<p class="muted small repos-discover-note">Algumas pastas não foram vistas a tempo. Se o macOS pediu acesso a uma pasta (por exemplo a Secretária), responda ao pedido e escolha «Procurar novamente».</p>`
        : "";

    sec.innerHTML = `
      <header class="repos-discover-head">
        <div>
          <h2 class="panel-title" id="repos-discover-title">Encontrados neste Mac</h2>
          <p class="muted small">Clones do GitHub que ainda não estão vinculados.${
            discovery.extraRoots.length
              ? ` Também em ${discovery.extraRoots.map((r) => `<span class="mono">${esc(shortPath(r))}</span>`).join(", ")}.`
              : ""
          }</p>
        </div>
        <div class="repos-discover-actions">
          <button type="button" class="btn ghost" data-action="discover-folder"${scanning ? " disabled" : ""}>${icon("folder")}<span>Procurar noutra pasta…</span></button>
          <button type="button" class="btn" data-action="discover"${scanning ? " disabled aria-busy=\"true\"" : ""}>${
            scanning ? `<span class="spinner" aria-hidden="true"></span><span>A procurar…</span>` : `${icon("refresh")}<span>${discovery.status === "idle" ? "Procurar" : "Procurar novamente"}</span>`
          }</button>
          ${
            discovery.items.length > 1
              ? `<button type="button" class="btn primary" data-action="bind-all"${ready.length ? "" : " disabled"}>${icon("plus")}<span>Vincular todos (${ready.length})</span></button>`
              : ""
          }
        </div>
      </header>
      ${body}
      ${truncatedNote}`;

    sec.querySelector<HTMLButtonElement>('[data-action="discover"]')?.addEventListener("click", () => void runDiscovery());
    sec.querySelector<HTMLButtonElement>('[data-action="discover-folder"]')?.addEventListener("click", () => void discoverFolder());
    sec.querySelector<HTMLButtonElement>('[data-action="bind-all"]')?.addEventListener("click", (e) =>
      void bindAll(e.currentTarget as HTMLButtonElement)
    );
    sec.querySelectorAll<HTMLLIElement>(".repos-discover-row").forEach((li) => {
      const path = li.dataset.path!;
      li.querySelector<HTMLSelectElement>("select")?.addEventListener("change", (e) => {
        const v = (e.target as HTMLSelectElement).value;
        if (v) discovery.choice.set(path, v);
        renderDiscovery();
        sec.querySelector<HTMLSelectElement>(`.repos-discover-row[data-path="${CSS.escape(path)}"] select`)?.focus();
      });
      li.querySelector<HTMLButtonElement>('[data-action="bind-found"]')?.addEventListener("click", (e) =>
        void bindFound(e.currentTarget as HTMLButtonElement, path)
      );
    });
  }

  function mergeDiscovered(items: DiscoveredRepo[]): void {
    const seen = new Set(discovery.items.map((d) => d.root_path));
    for (const d of items) if (!seen.has(d.root_path)) discovery.items.push(d);
    discovery.items.sort((a, b) => repoName(a).localeCompare(repoName(b)));
  }

  function runDiscovery(): Promise<void> {
    if (discovery.run) return discovery.run;
    discovery.status = "scanning";
    discovery.error = "";
    renderDiscovery();
    discovery.run = (async () => {
      try {
        const results = await Promise.all([
          reposApi.discover(null),
          ...(discovery.extraRoots.length ? [reposApi.discover(discovery.extraRoots)] : []),
        ]);
        discovery.items = [];
        results.forEach((r) => mergeDiscovered(r.repos));
        discovery.truncated = results.some((r) => r.truncated);
        discovery.status = "done";
      } catch (e) {
        discovery.status = "error";
        discovery.error = errMessage(e);
      } finally {
        discovery.run = null;
        onDiscoveryUpdate?.();
      }
    })();
    return discovery.run;
  }

  async function discoverFolder(): Promise<void> {
    let path: string | null;
    try {
      path = await reposApi.pickFolder();
    } catch (e) {
      showToast(errMessage(e), "err");
      return;
    }
    if (!path || disposed) return;
    if (!discovery.extraRoots.includes(path)) discovery.extraRoots.push(path);
    discovery.status = "scanning";
    renderDiscovery();
    try {
      const r = await reposApi.discover([path]);
      mergeDiscovered(r.repos);
      discovery.status = "done";
      if (r.repos.length === 0) showToast(`Não foram encontrados clones do GitHub por vincular em ${shortPath(path)}.`, "err");
    } catch (e) {
      discovery.status = "error";
      discovery.error = errMessage(e);
    }
    onDiscoveryUpdate?.();
  }

  async function bindFound(btn: HTMLButtonElement, path: string): Promise<void> {
    const d = discovery.items.find((x) => x.root_path === path);
    const accountId = d && choiceFor(d);
    if (!d || !accountId) return;
    try {
      const row = await withBusy(btn, "A vincular…", () => reposApi.bindPath(path, accountId));
      if (disposed) return;
      upsertRepo(row);
      forgetDiscovered(path);
      showToast(`${repoName(row)} vinculado a ${who(accountId)}.`, "ok");
      if (!rememberedSelection) rememberedSelection = row.id;
      render();
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  }

  async function bindAll(btn: HTMLButtonElement): Promise<void> {
    const items = discovery.items
      .map((d) => ({ path: d.root_path, account_id: choiceFor(d) }))
      .filter((x): x is { path: string; account_id: string } => !!x.account_id);
    if (items.length === 0) return;
    try {
      const results = await withBusy(btn, "A vincular…", () => reposApi.bindMany(items));
      if (disposed) return;
      const ok = results.filter((r) => r.repo);
      const failed = results.filter((r) => r.error);
      ok.forEach((r) => {
        upsertRepo(r.repo!);
        forgetDiscovered(r.path);
      });
      if (ok.length) showToast(ok.length === 1 ? "1 repositório vinculado." : `${ok.length} repositórios vinculados.`, "ok");
      if (failed.length) {
        showToast(
          `Não foi possível vincular ${failed.length === 1 ? "1 repositório" : `${failed.length} repositórios`}:\n${failed
            .map((f) => `${shortPath(f.path)} — ${f.error}`)
            .join("\n")}`,
          "err"
        );
      }
      render();
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  }

  // ---------- Carregamento ----------

  async function load(): Promise<void> {
    const token = ++loadToken;
    if (!root.querySelector(".repos-layout, .empty-state")) renderLoading();
    try {
      const [accs, rows] = await Promise.all([api.accounts(), reposApi.list()]);
      if (disposed || token !== loadToken) return;
      accounts = accs;
      repos = rows;
      const hadFocusInList = root.querySelector(".repos-list")?.contains(document.activeElement) ?? false;
      render();
      if (hadFocusInList) focusSelected();
      if (accounts.length > 0 && discovery.status === "idle") void runDiscovery();
    } catch (e) {
      if (disposed || token !== loadToken) return;
      renderError(errMessage(e));
    }
  }

  onDiscoveryUpdate = () => {
    if (!disposed) renderDiscovery();
  };
  const onDataChanged = (): void => void load();
  document.addEventListener(DATA_CHANGED_EVENT, onDataChanged);
  void load();

  return () => {
    disposed = true;
    closeMenu?.();
    onDiscoveryUpdate = null;
    document.removeEventListener(DATA_CHANGED_EVENT, onDataChanged);
  };
};
