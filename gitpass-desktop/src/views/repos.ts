import "./repos.css";
import { api, type AccountRow, type BindingRow } from "../lib/api";
import { errMessage, esc, shortenPath, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileChip, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView } from "../lib/view";

/** Seleção mantida em memória entre recarregamentos e mudanças de ecrã. */
let rememberedSelection: string | null = null;

type ProfileRef = Pick<AccountRow, "id" | "login" | "label">;

function repoName(b: BindingRow): string {
  if (b.github_owner && b.github_repo) return `${b.github_owner}/${b.github_repo}`;
  const parts = b.root_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? b.root_path;
}

const shortPath = shortenPath;

function githubUrl(b: BindingRow): string | null {
  if (!b.github_owner || !b.github_repo) return null;
  return `https://github.com/${encodeURIComponent(b.github_owner)}/${encodeURIComponent(b.github_repo)}`;
}


function parseCommit(line: string): { hash: string | null; message: string } {
  const m = /^([0-9a-f]{7,40})\b\s*(.*)$/i.exec(line.trim());
  return m ? { hash: m[1], message: m[2] } : { hash: null, message: line };
}

export const mount: MountView = (root, ctx) => {
  let accounts: AccountRow[] = [];
  let bindings: BindingRow[] = [];
  let query = "";
  let loadToken = 0;
  let logToken = 0;
  let disposed = false;
  let closeMenu: (() => void) | null = null;

  const profileFor = (b: BindingRow): ProfileRef => {
    const acc = accounts.find((a) => a.id === b.account_id);
    if (acc) return acc;
    return { id: b.account_id, login: b.account_login, label: null };
  };

  const filtered = (): BindingRow[] => {
    const q = query.trim().toLowerCase();
    if (!q) return bindings;
    return bindings.filter((b) =>
      [repoName(b), b.root_path, profileName(profileFor(b)), b.account_login ?? ""].some((s) =>
        s.toLowerCase().includes(q)
      )
    );
  };

  const bindLabel = (): string => (accounts.length === 0 ? "Ligar uma conta primeiro" : "Vincular repositório");

  const bindButton = (extra = ""): string =>
    `<button type="button" class="btn primary ${extra}" data-action="bind" aria-haspopup="${
      accounts.length > 1 ? "menu" : "false"
    }" aria-expanded="false">${icon(accounts.length === 0 ? "user" : "plus")}<span>${esc(bindLabel())}</span></button>`;

  const subtitle = (): string => {
    const n = bindings.length;
    if (n === 0) return "Nenhum repositório local vinculado";
    return n === 1 ? "1 repositório local vinculado" : `${n} repositórios locais vinculados`;
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

    if (bindings.length === 0) {
      root.innerHTML = `${header}
        <section class="panel">
          <div class="empty-state">
            <div class="empty-icon">${icon("repo", 26)}</div>
            <h3>Nenhum repositório vinculado</h3>
            <p>Vincule uma pasta local com um repositório GitHub a uma das suas contas. O GitPass passa a usar automaticamente as credenciais certas nesse repositório.</p>
            <div class="repos-actions">${bindButton("lg")}</div>
          </div>
        </section>`;
      wireBindButtons();
      return;
    }

    if (!rememberedSelection || !bindings.some((b) => b.id === rememberedSelection)) {
      rememberedSelection = bindings[0].id;
    }

    root.innerHTML = `${header}
      <div class="repos-layout">
        <section class="repos-list-pane" aria-label="Lista de repositórios">
          <label class="repos-search">
            <span class="sr-only">Procurar repositórios</span>
            ${icon("search")}
            <input type="search" class="input" placeholder="Procurar por repositório, pasta ou perfil" value="${esc(query)}" autocomplete="off" spellcheck="false" />
          </label>
          <ul class="list repos-list" role="listbox" aria-label="Repositórios vinculados"></ul>
        </section>
        <section class="panel repos-detail" aria-live="polite"></section>
      </div>`;

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
      const li = (e.target as HTMLElement).closest<HTMLLIElement>("[data-id]");
      if (li?.dataset.id) select(li.dataset.id, true);
    });
    list.addEventListener("keydown", onListKey);

    wireBindButtons();
    renderList();
    renderDetail();
  }

  function renderList(): void {
    const list = root.querySelector<HTMLUListElement>(".repos-list");
    if (!list) return;
    const items = filtered();
    if (items.length === 0) {
      list.innerHTML = `<li class="repos-no-results muted small" role="presentation">${icon("search")} Nenhum resultado para «${esc(query.trim())}».</li>`;
      return;
    }
    const selectedVisible = items.some((b) => b.id === rememberedSelection);
    list.innerHTML = items
      .map((b, i) => {
        const sel = b.id === rememberedSelection;
        const focusable = sel || (!selectedVisible && i === 0);
        return `<li class="list-row repos-row" role="option" id="repos-opt-${esc(b.id)}" data-id="${esc(b.id)}" aria-selected="${sel}" tabindex="${focusable ? 0 : -1}">
          <span class="repos-row-main">
            <span class="mono repos-row-name truncate">${esc(repoName(b))}</span>
            <span class="mono muted small repos-row-path truncate" title="${esc(b.root_path)}">${esc(shortPath(b.root_path))}</span>
          </span>
          ${profileChip(profileFor(b))}
        </li>`;
      })
      .join("");
  }

  function focusSelected(): void {
    const list = root.querySelector<HTMLUListElement>(".repos-list");
    list?.querySelector<HTMLLIElement>('[tabindex="0"]')?.focus();
  }

  function onListKey(e: KeyboardEvent): void {
    const items = filtered();
    if (items.length === 0) return;
    const cur = (document.activeElement as HTMLElement | null)?.closest<HTMLLIElement>("[data-id]")?.dataset.id;
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
      case "Enter":
      case " ":
        if (cur) {
          e.preventDefault();
          select(cur, true);
        }
        return;
      default:
        return;
    }
    e.preventDefault();
    select(items[Math.max(0, idx)].id, true);
  }

  function select(id: string, focus: boolean): void {
    const changed = id !== rememberedSelection;
    rememberedSelection = id;
    const list = root.querySelector<HTMLUListElement>(".repos-list");
    list?.querySelectorAll<HTMLLIElement>("[data-id]").forEach((li) => {
      const sel = li.dataset.id === id;
      li.setAttribute("aria-selected", String(sel));
      li.tabIndex = sel ? 0 : -1;
      if (sel && focus) {
        li.focus();
        li.scrollIntoView({ block: "nearest" });
      }
    });
    if (changed) renderDetail();
  }

  function renderDetail(): void {
    const panel = root.querySelector<HTMLElement>(".repos-detail");
    if (!panel) return;
    const b = bindings.find((x) => x.id === rememberedSelection);
    if (!b) {
      panel.innerHTML = `<p class="muted small">Selecione um repositório.</p>`;
      return;
    }
    const url = githubUrl(b);
    panel.innerHTML = `
      <div class="repos-detail-head">
        <div class="repos-detail-title">
          <span class="repos-detail-icon">${icon("repo", 20)}</span>
          <h2 class="panel-title mono truncate" title="${esc(repoName(b))}">${esc(repoName(b))}</h2>
        </div>
        ${profileChip(profileFor(b), { size: "md" })}
      </div>
      <dl class="repos-meta">
        <div class="repos-meta-item">
          <dt class="field-label">Pasta local</dt>
          <dd class="repos-path-row">
            <code class="repos-path mono">${esc(b.root_path)}</code>
            <button type="button" class="btn ghost icon-only" data-action="copy-path" aria-label="Copiar caminho" title="Copiar caminho">${icon("copy")}</button>
          </dd>
        </div>
        <div class="repos-meta-item">
          <dt class="field-label">GitHub</dt>
          <dd>${
            url
              ? `<a class="repos-gh-link mono" href="${esc(url)}" target="_blank" rel="noopener noreferrer">${esc(
                  `github.com/${b.github_owner}/${b.github_repo}`
                )} ${icon("external", 14)}<span class="sr-only">(abre num novo separador)</span></a>`
              : `<span class="badge warning">${icon("alert", 12)} Origin desconhecido</span>`
          }</dd>
        </div>
      </dl>
      <h3 class="section-label repos-log-label">${icon("commit", 14)} Histórico de commits</h3>
      <div class="repos-log"></div>`;

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

    void loadLog(b, panel.querySelector<HTMLElement>(".repos-log")!);
  }

  async function loadLog(b: BindingRow, box: HTMLElement): Promise<void> {
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
      box.innerHTML = `<div class="callout error" role="alert">${icon("alert")}<span>Não foi possível ler o histórico: ${esc(
        errMessage(e)
      )}</span></div>`;
    }
  }

  function wireBindButtons(): void {
    root.querySelectorAll<HTMLButtonElement>('[data-action="bind"]').forEach((btn) => {
      btn.addEventListener("click", () => {
        if (accounts.length === 0) {
          ctx.navigate("accounts");
        } else if (accounts.length === 1) {
          void bind(btn, accounts[0].id);
        } else if (closeMenu) {
          closeMenu();
        } else {
          openMenu(btn);
        }
      });
    });
  }

  function openMenu(btn: HTMLButtonElement): void {
    const host = btn.parentElement!;
    const menu = document.createElement("div");
    menu.className = "material repos-menu";
    menu.setAttribute("role", "menu");
    menu.setAttribute("aria-label", "Escolher conta para o vínculo");
    menu.innerHTML = `<p class="repos-menu-title muted small">Vincular com a conta</p>${accounts
      .map(
        (a) =>
          `<button type="button" class="repos-menu-item" role="menuitem" data-account="${esc(a.id)}">${profileChip(a)}${
            a.login && a.label ? `<span class="mono muted small truncate">@${esc(a.login)}</span>` : ""
          }</button>`
      )
      .join("")}`;
    host.classList.add("repos-menu-host");
    host.appendChild(menu);
    btn.setAttribute("aria-expanded", "true");
    const items = Array.from(menu.querySelectorAll<HTMLButtonElement>(".repos-menu-item"));
    items[0]?.focus();

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
      if (!item?.dataset.account) return;
      const id = item.dataset.account;
      close();
      void bind(btn, id);
    });
    closeMenu = close;
  }

  async function bind(btn: HTMLButtonElement, accountId: string): Promise<void> {
    try {
      await withBusy(btn, "A vincular…", () => api.bindRepoDialog(accountId));
      if (disposed) return;
      showToast("Repositório vinculado com sucesso.", "ok");
      ctx.notifyDataChanged();
      await load();
    } catch (e) {
      showToast(errMessage(e) || "Não foi possível vincular o repositório.", "err");
    }
  }

  async function load(): Promise<void> {
    const token = ++loadToken;
    if (!root.querySelector(".repos-layout, .empty-state")) renderLoading();
    try {
      const [accs, binds] = await Promise.all([api.accounts(), api.bindings()]);
      if (disposed || token !== loadToken) return;
      accounts = accs;
      bindings = binds;
      const hadFocusInList = root.querySelector(".repos-list")?.contains(document.activeElement) ?? false;
      render();
      if (hadFocusInList) focusSelected();
    } catch (e) {
      if (disposed || token !== loadToken) return;
      renderError(errMessage(e));
    }
  }

  const onDataChanged = (): void => void load();
  document.addEventListener(DATA_CHANGED_EVENT, onDataChanged);
  void load();

  return () => {
    disposed = true;
    closeMenu?.();
    document.removeEventListener(DATA_CHANGED_EVENT, onDataChanged);
  };
};
