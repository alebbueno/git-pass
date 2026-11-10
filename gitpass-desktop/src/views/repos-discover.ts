import type { AccountRow } from "../lib/api";
import { reposApi, type DiscoveredRepo, type RepoRow } from "../lib/api-repos";
import { errMessage, esc, shortenPath, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileName } from "../lib/profile";
import { REASON_TEXT, suggestionMotivo, repoName, whoText } from "./repos-ui";

type DiscoveryStatus = "idle" | "scanning" | "done" | "error";

/** Resultados de «Encontrados neste Mac», mantidos durante a sessão (a procura roda uma vez automaticamente). */
const discovery = {
  status: "idle" as DiscoveryStatus,
  items: [] as DiscoveredRepo[],
  error: "",
  truncated: false,
  extraRoots: [] as string[],
  /** Conta escolhida por item (caminho → id da conta). */
  choice: new Map<string, string>(),
  run: null as Promise<void> | null,
  /** `null` até o usuário abrir ou fechar a seção. */
  open: null as boolean | null,
};

/** Re-renderiza a seção montada (quando a procura termina fora do ciclo da tela). */
let onDiscoveryUpdate: (() => void) | null = null;

export type DiscoveryDeps = {
  accounts: () => AccountRow[];
  repos: () => RepoRow[];
  /** Um ou mais repositórios foram vinculados aqui. */
  onBound: (rows: RepoRow[]) => void;
  isDisposed: () => boolean;
  /** A procura terminou ou mudou a lista. */
  onUpdate?: () => void;
};

export type DiscoveryHandle = {
  /** Renderiza em `section` (ou na última usada). `defaultOpen` vale até o usuário abrir/fechar. */
  render: (section?: HTMLElement | null, defaultOpen?: boolean) => void;
  /** Roda a procura automática uma vez por sessão. */
  autoRun: () => void;
  forget: (path: string) => void;
  /** Quantos clones sem vínculo foram encontrados. */
  count: () => number;
  /** Abre a seção (atalho «Encontrados neste Mac» do cabeçalho). */
  open: () => void;
  dispose: () => void;
};

export function createDiscovery(deps: DiscoveryDeps): DiscoveryHandle {
  let sec: HTMLElement | null = null;
  let defaultOpen = false;

  const who = (id: string): string => whoText(deps.accounts(), id);

  function forget(path: string): void {
    discovery.items = discovery.items.filter((d) => d.root_path !== path);
    discovery.choice.delete(path);
  }

  function choiceFor(d: DiscoveredRepo): string | null {
    const c = discovery.choice.get(d.root_path) ?? d.suggested_account_id;
    return c && deps.accounts().some((a) => a.id === c) ? c : null;
  }

  function render(section?: HTMLElement | null, open?: boolean): void {
    if (section !== undefined) sec = section;
    if (open !== undefined) defaultOpen = open;
    if (!sec || !sec.isConnected) return;
    const accounts = deps.accounts();
    const bound = new Set(deps.repos().map((r) => r.root_path));
    discovery.items = discovery.items.filter((d) => !bound.has(d.root_path));
    const ready = discovery.items.filter((d) => choiceFor(d));
    const scanning = discovery.status === "scanning";
    const focusKey = sec.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.focus : undefined;

    let body: string;
    if (scanning && discovery.items.length === 0) {
      body = `<div class="repos-discover-list" aria-busy="true"><span class="sr-only">Procurando repositórios…</span>${Array.from(
        { length: 3 },
        () => `<div class="skeleton repos-skeleton-row"></div>`
      ).join("")}</div>`;
    } else if (discovery.status === "error") {
      body = `<div class="callout error" role="alert">${icon("alert")}<span>Não foi possível procurar repositórios: ${esc(discovery.error)}</span></div>`;
    } else if (discovery.items.length === 0) {
      body = `<p class="muted small repos-discover-empty">${icon("check")} ${
        discovery.status === "done"
          ? "Não há clones do GitHub sem vínculo nas pastas de sempre (Developer, Projects, GitHub, Code, src, Documentos/GitHub, Mesa)."
          : "Procure clones do GitHub que já existam neste Mac."
      }</p>`;
    } else {
      body = `<ul class="list repos-discover-list">${discovery.items
        .map((d, i) => {
          const chosen = choiceFor(d);
          const name = `${d.github_owner}/${d.github_repo}`;
          const reason =
            chosen && chosen === d.suggested_account_id && (d.suggestion_reason || d.suggestion_label)
              ? `<span class="muted small">Sugerida: ${esc(suggestionMotivo(d.suggestion_reason, d.suggestion_label))}</span>`
              : !chosen
                ? `<span class="warn small">${icon("alert", 12)}Escolha a conta</span>`
                : "";
          return `<li class="list-row repos-discover-row" data-path="${esc(d.root_path)}">
            <span class="repos-discover-main">
              <span class="mono repos-discover-name truncate">${esc(name)}</span>
              <span class="mono muted small truncate" title="${esc(d.root_path)}">${esc(shortenPath(d.root_path))}</span>
            </span>
            <span class="repos-discover-pick">
              <select class="select repos-discover-select" data-focus="sel-${i}" aria-label="Conta para ${esc(name)}">
                ${chosen ? "" : `<option value="" selected>Escolher conta…</option>`}
                ${accounts
                  .map(
                    (a) =>
                      `<option value="${esc(a.id)}"${a.id === chosen ? " selected" : ""}>${esc(
                        a.login ? `${profileName(a)} (@${a.login})` : profileName(a)
                      )}</option>`
                  )
                  .join("")}
              </select>
              ${reason}
            </span>
            <button type="button" class="btn" data-action="bind-found" data-focus="bind-${i}"${chosen ? "" : " disabled"}>${icon("plus", 14)}<span>Vincular</span></button>
          </li>`;
        })
        .join("")}</ul>`;
    }

    const truncatedNote =
      discovery.truncated && discovery.status === "done"
        ? `<p class="muted small repos-discover-note">Algumas pastas demoraram demais e não foram verificadas. Se o macOS pediu acesso a uma pasta (como a Mesa), permita e clique em “Procurar de novo”.</p>`
        : "";

    const isOpen = discovery.open ?? (defaultOpen || discovery.items.length > 0);
    const count = discovery.items.length
      ? `<span class="badge repos-discover-count">${esc(countLabel(discovery.items.length))}</span>`
      : scanning
        ? `<span class="badge repos-discover-count"><span class="spinner" aria-hidden="true"></span> Procurando…</span>`
        : "";

    sec.innerHTML = `
      <details class="repos-discover-details"${isOpen ? " open" : ""}>
        <summary class="repos-discover-summary" data-focus="summary">
          <span class="repos-discover-chevron" aria-hidden="true">${icon("chevronRight", 14)}</span>
          <span class="repos-discover-title" id="repos-discover-title">Encontrados neste Mac</span>
          ${count}
        </summary>
        <div class="repos-discover-content">
          <p class="muted small repos-discover-lead">Clones do GitHub que ainda não estão vinculados.${
            discovery.extraRoots.length
              ? ` Também em ${discovery.extraRoots.map((r) => `<span class="mono">${esc(shortenPath(r))}</span>`).join(", ")}.`
              : ""
          }</p>
          <div class="repos-discover-actions">
            <button type="button" class="btn ghost" data-action="discover-folder" data-focus="folder"${scanning ? " disabled" : ""}>${icon(
              "folder"
            )}<span>Procurar em outra pasta…</span></button>
            <button type="button" class="btn" data-action="discover" data-focus="discover"${scanning ? ' disabled aria-busy="true"' : ""}>${
              scanning
                ? `<span class="spinner" aria-hidden="true"></span><span>Procurando…</span>`
                : `${icon("refresh")}<span>${discovery.status === "idle" ? "Procurar" : "Procurar de novo"}</span>`
            }</button>
            ${
              discovery.items.length > 1
                ? `<button type="button" class="btn primary" data-action="bind-all" data-focus="all"${ready.length ? "" : " disabled"}>${icon(
                    "plus"
                  )}<span>Vincular todos (${ready.length})</span></button>`
                : ""
            }
          </div>
          ${body}
          ${truncatedNote}
        </div>
      </details>`;

    const details = sec.querySelector<HTMLDetailsElement>("details")!;
    details.addEventListener("toggle", () => {
      discovery.open = details.open;
    });
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
        render();
      });
      li.querySelector<HTMLButtonElement>('[data-action="bind-found"]')?.addEventListener("click", (e) =>
        void bindFound(e.currentTarget as HTMLButtonElement, path)
      );
    });
    if (focusKey) sec.querySelector<HTMLElement>(`[data-focus="${CSS.escape(focusKey)}"]`)?.focus();
  }

  function countLabel(n: number): string {
    return n === 1 ? "1 sem vínculo" : `${n} sem vínculo`;
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
    render();
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
    if (!path || deps.isDisposed()) return;
    if (!discovery.extraRoots.includes(path)) discovery.extraRoots.push(path);
    discovery.status = "scanning";
    render();
    try {
      const r = await reposApi.discover([path]);
      mergeDiscovered(r.repos);
      discovery.status = "done";
      if (r.repos.length === 0) showToast(`Não encontramos clones do GitHub sem vínculo em ${shortenPath(path)}.`, "err");
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
      const row = await withBusy(btn, "Vinculando…", () => reposApi.bindPath(path, accountId));
      if (deps.isDisposed()) return;
      forget(path);
      showToast(`${repoName(row)} vinculado a ${who(accountId)}.`, "ok");
      deps.onBound([row]);
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
      const results = await withBusy(btn, "Vinculando…", () => reposApi.bindMany(items));
      if (deps.isDisposed()) return;
      const ok = results.filter((r) => r.repo);
      const failed = results.filter((r) => r.error);
      ok.forEach((r) => forget(r.path));
      if (ok.length) showToast(ok.length === 1 ? "1 repositório vinculado." : `${ok.length} repositórios vinculados.`, "ok");
      if (failed.length) {
        showToast(
          `Não foi possível vincular ${failed.length === 1 ? "1 repositório" : `${failed.length} repositórios`}:\n${failed
            .map((f) => `${shortenPath(f.path)} — ${f.error}`)
            .join("\n")}`,
          "err"
        );
      }
      deps.onBound(ok.map((r) => r.repo!));
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  }

  onDiscoveryUpdate = () => {
    if (!deps.isDisposed()) render();
    deps.onUpdate?.();
  };

  return {
    render,
    autoRun: () => {
      if (deps.accounts().length > 0 && discovery.status === "idle") void runDiscovery();
    },
    forget,
    count: () => discovery.items.length,
    open: () => {
      discovery.open = true;
      const details = sec?.querySelector<HTMLDetailsElement>("details");
      if (details) details.open = true;
      else render();
      if (discovery.status === "idle" && deps.accounts().length > 0) void runDiscovery();
    },
    dispose: () => {
      onDiscoveryUpdate = null;
    },
  };
}
