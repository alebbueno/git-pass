import "./repo-branches.css";
import { reposApi, type BranchInfo, type RepoRow } from "../lib/api-repos";
import { errMessage, esc, relativeTime, shortenPath, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { DATA_CHANGED_EVENT } from "../lib/view";
import { plural, repoName } from "./repos-ui";

// ---------- Branches por repositório (mantidos entre telas) ----------

/** Quantos commits um branch tem que o padrão não tem (prévia de «juntar», sem alterar nada). */
type AheadOfDefault = { key: string; status: "loading" | "ok" | "error"; count: number; truncated: boolean };

type Store = {
  id: string;
  list: BranchInfo[] | null;
  error: string | null;
  checkError: string | null;
  checkedAt: number | null;
  pendingLocal: number;
  pendingFetch: number;
  seq: number;
  appliedSeq: number;
  vsDefault: Map<string, AheadOfDefault>;
  scanning: boolean;
};

type Listener = (id: string) => void;

const stores = new Map<string, Store>();
const listeners = new Set<Listener>();

/** Só os branches mais recentes são comparados com o padrão, para não pesar em repositórios com muitos branches. */
const MAX_COMPARE = 10;

function emit(id: string): void {
  for (const l of listeners) l(id);
}

export function onBranchesChange(l: Listener): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function branchStore(id: string): Store {
  let s = stores.get(id);
  if (!s) {
    s = {
      id,
      list: null,
      error: null,
      checkError: null,
      checkedAt: null,
      pendingLocal: 0,
      pendingFetch: 0,
      seq: 0,
      appliedSeq: 0,
      vsDefault: new Map(),
      scanning: false,
    };
    stores.set(id, s);
  }
  return s;
}

export function forgetBranches(id: string): void {
  stores.delete(id);
}

export function branchesOf(id: string): BranchInfo[] | null {
  return stores.get(id)?.list ?? null;
}

export function defaultBranchOf(id: string): BranchInfo | null {
  return stores.get(id)?.list?.find((b) => b.is_default) ?? null;
}

export function aheadOfDefault(id: string, b: BranchInfo): AheadOfDefault | null {
  const s = stores.get(id);
  const def = defaultBranchOf(id);
  if (!s || !def) return null;
  const e = s.vsDefault.get(b.name);
  return e && e.key === compareKey(b, def) ? e : null;
}

function compareKey(b: BranchInfo, def: BranchInfo): string {
  return `${b.head_sha}|${def.name}|${def.head_sha}`;
}

export async function refreshBranches(id: string, fetch: boolean): Promise<boolean> {
  const s = branchStore(id);
  const seq = ++s.seq;
  if (fetch) s.pendingFetch++;
  else s.pendingLocal++;
  emit(id);
  let ok = false;
  try {
    const list = await reposApi.branches(id, fetch);
    if (seq > s.appliedSeq) {
      s.appliedSeq = seq;
      s.list = list;
      s.error = null;
      if (fetch) {
        s.checkError = null;
        s.checkedAt = Date.now();
      }
    }
    ok = true;
  } catch (e) {
    if (seq > s.appliedSeq) {
      if (fetch && s.list) s.checkError = errMessage(e);
      else s.error = errMessage(e);
    }
  } finally {
    if (fetch) s.pendingFetch--;
    else s.pendingLocal--;
    emit(id);
  }
  if (ok) void compareWithDefault(id);
  return ok;
}

/** Carrega a lista local uma vez (sem rede), para quem só precisa saber o branch padrão. */
export function ensureBranches(id: string): void {
  const s = branchStore(id);
  if (s.list || s.pendingLocal > 0 || s.pendingFetch > 0 || s.error) return;
  void refreshBranches(id, false);
}

/** Um de cada vez: cada prévia lê o repositório inteiro. */
async function compareWithDefault(id: string): Promise<void> {
  const s = branchStore(id);
  if (s.scanning) return;
  s.scanning = true;
  try {
    for (;;) {
      const def = defaultBranchOf(id);
      if (!def || stores.get(id) !== s) return;
      const next = (s.list ?? [])
        .filter((b) => !b.is_default && !b.remote_only)
        .slice(0, MAX_COMPARE)
        .find((b) => s.vsDefault.get(b.name)?.key !== compareKey(b, def));
      if (!next) return;
      const key = compareKey(next, def);
      s.vsDefault.set(next.name, { key, status: "loading", count: 0, truncated: false });
      try {
        const p = await reposApi.mergePreview(id, next.name, def.name, "merge", []);
        s.vsDefault.set(next.name, { key, status: "ok", count: p.commits.length, truncated: p.commits.length >= 200 });
      } catch {
        s.vsDefault.set(next.name, { key, status: "error", count: 0, truncated: false });
      }
      emit(id);
    }
  } finally {
    s.scanning = false;
  }
}

let globalReady = false;
let dataTimer: number | undefined;

/** Recarrega (sem rede) os repositórios que já têm branches carregados quando algo muda fora daqui. */
function ensureGlobalListeners(): void {
  if (globalReady) return;
  globalReady = true;
  reposApi
    .onOutgoingChanged((id) => {
      if (stores.get(id)?.list) void refreshBranches(id, false);
    })
    .catch(() => {
      globalReady = false;
    });
  document.addEventListener(DATA_CHANGED_EVENT, () => {
    window.clearTimeout(dataTimer);
    dataTimer = window.setTimeout(() => {
      for (const s of stores.values()) if (s.list && s.pendingLocal === 0) void refreshBranches(s.id, false);
    }, 250);
  });
}

// ---------- Aba «Branches» ----------

export type BranchesTabDeps = {
  repo: () => RepoRow;
  /** Abre «Levar commits para outro branch» a partir de `source`. */
  openMerge: (source: string, target: string | null) => void;
  /** Abre a aba Enviar para `branch`. */
  openPush: (branch: string) => void;
  onCheckedOut: (row: RepoRow) => void;
};

const STALE_MS = 5 * 60_000;

export function mountBranchesTab(el: HTMLElement, deps: BranchesTabDeps): { refresh: () => void; dispose: () => void } {
  ensureGlobalListeners();
  let disposed = false;
  let prevChecking: boolean | null = null;
  let pendingFocus: string | null = null;
  el.innerHTML = `<p class="sr-only" role="status" aria-live="polite" data-ref="live"></p><div class="branches-root"></div>`;
  const live = el.querySelector<HTMLElement>('[data-ref="live"]')!;
  const body = el.querySelector<HTMLElement>(".branches-root")!;

  const announce = (text: string): void => {
    live.textContent = "";
    window.setTimeout(() => {
      live.textContent = text;
    }, 30);
  };

  function render(): void {
    const repo = deps.repo();
    const s = branchStore(repo.id);
    const active = document.activeElement as HTMLElement | null;
    const focusKey = active && body.contains(active) ? active.dataset.focus : undefined;
    body.innerHTML = html(repo, s);
    const checking = s.pendingFetch > 0;
    if (prevChecking !== null && prevChecking !== checking) {
      if (checking) announce("Verificando no GitHub…");
      else if (s.checkError) announce("Não deu para verificar no GitHub. Mostrando o que está neste Mac.");
      else if (s.list) announce(`Verificado no GitHub. ${plural(s.list.length, "branch", "branches")}.`);
    }
    prevChecking = checking;
    const key = pendingFocus ?? focusKey;
    pendingFocus = null;
    if (key) body.querySelector<HTMLElement>(`[data-focus="${CSS.escape(key)}"]`)?.focus({ preventScroll: !!focusKey });
  }

  function statusHtml(s: Store): string {
    const checking = s.pendingFetch > 0;
    let text: string;
    if (checking) text = `<span class="spinner" aria-hidden="true"></span><span>Verificando no GitHub…</span>`;
    else if (s.checkError)
      text = `${icon("alert", 14)}<span>Não deu para verificar no GitHub: ${esc(s.checkError)} Mostrando o que está neste Mac.</span>`;
    else if (s.checkedAt)
      text = `${icon("check", 14)}<span>Verificado no GitHub às ${esc(
        new Date(s.checkedAt).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })
      )}</span>`;
    else text = `${icon("info", 14)}<span>Mostrando o que está neste Mac. Ainda falta verificar no GitHub.</span>`;
    return `<div class="push-status${s.checkError ? " warn" : ""}">
      <p class="push-status-text small">${text}</p>
      <button type="button" class="btn ghost" data-action="branches-recheck" data-focus="recheck"${checking ? " disabled" : ""}>${icon(
        "refresh",
        14
      )}<span>Verificar de novo</span></button>
    </div>`;
  }

  function html(repo: RepoRow, s: Store): string {
    if (!repo.exists) {
      return `<div class="callout error" role="alert">${icon("alert")}<span>A pasta não foi encontrada neste Mac. Não dá para ver os branches.</span></div>`;
    }
    if (!s.list) {
      if (s.error) {
        return `<div class="callout error push-callout-action" role="alert">${icon("alert")}<span>Não foi possível ler os branches: ${esc(s.error)}</span>
          <button type="button" class="btn" data-action="branches-reload" data-focus="reload">${icon("refresh", 14)}<span>Tentar de novo</span></button></div>`;
      }
      return `<div class="push-loading" aria-busy="true"><span class="sr-only">Lendo os branches…</span>
        ${Array.from({ length: 4 }, () => `<div class="skeleton push-skeleton-row"></div>`).join("")}
      </div>`;
    }
    const def = s.list.find((b) => b.is_default) ?? null;
    const dirty = !!repo.sync?.dirty;
    const dirtyNote = dirty
      ? `<p class="callout warning branches-dirty" id="branches-dirty-note">${icon("edit", 14)}<span>Há alterações sem commit na pasta. Para mudar de branch, faça commit ou guarde (git stash) essas alterações antes.</span></p>`
      : "";
    const rows = s.list.map((b) => rowHtml(repo, b, def, dirty)).join("");
    return `<div class="branches">
      ${statusHtml(s)}
      ${dirtyNote}
      ${
        s.list.length === 0
          ? `<p class="muted small repo-empty-line">${icon("branch")} Este repositório ainda não tem branches.</p>`
          : `<ul class="branches-list" aria-label="${esc(`Branches de ${repoName(repo)}`)}">${rows}</ul>`
      }
    </div>`;
  }

  function rowHtml(repo: RepoRow, b: BranchInfo, def: BranchInfo | null, dirty: boolean): string {
    const n = esc(b.name);
    const badges: string[] = [];
    if (b.current) badges.push(`<span class="badge branches-badge-current">${icon("check", 12)} Atual</span>`);
    if (b.is_default) badges.push(`<span class="badge">Padrão</span>`);
    if (b.remote_only) badges.push(`<span class="badge">${icon("cloud", 12)} Só no GitHub</span>`);
    else if (!b.upstream) badges.push(`<span class="badge">Ainda não está no GitHub</span>`);
    if (b.worktree_path && !b.current)
      badges.push(`<span class="badge warning" title="${esc(b.worktree_path)}">${icon("folder", 12)} Aberto em outra pasta</span>`);
    if (b.ahead) badges.push(`<span class="badge repo-badge-ahead">${icon("arrowUp", 12)} ${esc(`${b.ahead.toLocaleString("pt-BR")} para enviar`)}</span>`);
    if (b.behind)
      badges.push(
        `<span class="badge">${icon("arrowDown", 12)} ${esc(`${b.behind.toLocaleString("pt-BR")} atrás`)}<span class="sr-only"> do GitHub</span></span>`
      );
    const vs = def && !b.is_default && !b.remote_only ? aheadOfDefault(repo.id, b) : null;
    const hasNew = vs?.status === "ok" && vs.count > 0;
    if (def && vs?.status === "ok") {
      badges.push(
        vs.count > 0
          ? `<span class="badge branches-badge-new">${esc(`${vs.count.toLocaleString("pt-BR")}${vs.truncated ? "+" : ""} que ${def.name} não tem`)}</span>`
          : `<span class="badge success">${icon("check", 12)} ${esc(`Já está em ${def.name}`)}</span>`
      );
    }

    const actions: string[] = [];
    let note = "";
    const rowKey = `b-${b.name}`;
    if (!b.is_default && !b.remote_only) {
      const label = def ? `Levar para ${def.name}…` : "Levar para outro branch…";
      actions.push(
        `<button type="button" class="btn${hasNew ? " primary" : ""}" data-action="branch-merge" data-branch="${n}" data-focus="${esc(
          `${rowKey}-merge`
        )}">${icon("merge", 14)}<span>${esc(label)}</span></button>`
      );
    }
    if (!b.current) {
      const reasonId = `branch-reason-${esc(b.name.replace(/[^A-Za-z0-9_-]/g, "_"))}`;
      const blocked = dirty ? "branches-dirty-note" : b.worktree_path ? reasonId : null;
      actions.push(
        `<button type="button" class="btn" data-action="branch-checkout" data-branch="${n}" data-focus="${esc(`${rowKey}-checkout`)}"${
          blocked ? ` disabled aria-describedby="${blocked}"` : ""
        }>${icon("branch", 14)}<span>Mudar para este branch</span></button>`
      );
      if (b.worktree_path && !dirty)
        note = `<span class="small muted branches-reason" id="${reasonId}">Aberto em <span class="mono">${esc(
          shortenPath(b.worktree_path)
        )}</span>. Mude por lá ou feche aquela pasta.</span>`;
    }
    if (!b.remote_only) {
      actions.push(
        `<button type="button" class="btn ghost" data-action="branch-push" data-branch="${n}" data-focus="${esc(`${rowKey}-push`)}">${icon(
          "send",
          14
        )}<span>Enviar</span></button>`
      );
    }
    if (note) actions.push(note);

    const when = b.last_commit_at
      ? ` · <time datetime="${esc(b.last_commit_at)}" title="${esc(new Date(b.last_commit_at).toLocaleString("pt-BR"))}">${esc(relativeTime(b.last_commit_at))}</time>`
      : "";
    return `<li class="branches-row${b.current ? " current" : ""}${hasNew ? " has-new" : ""}" data-branch="${n}">
      <div class="branches-main">
        <p class="branches-name"><span class="mono">${n}</span>${badges.join("")}</p>
        <p class="branches-last small muted">${icon("commit", 12)}<span class="branches-subject">${esc(b.last_commit_subject || "(sem mensagem)")}</span>${when}</p>
      </div>
      <div class="branches-actions" role="group" aria-label="${esc(`Ações do branch ${b.name}`)}">${actions.join("")}</div>
    </li>`;
  }

  async function checkout(btn: HTMLButtonElement, name: string): Promise<void> {
    const repo = deps.repo();
    try {
      const row = await withBusy(btn, "Mudando…", () => reposApi.checkout(repo.id, name));
      if (disposed) return;
      showToast(`Agora a pasta está no branch ${name}.`, "ok");
      announce(`Agora a pasta está no branch ${name}.`);
      pendingFocus = `b-${name}-push`;
      deps.onCheckedOut(row);
      void refreshBranches(repo.id, false);
    } catch (e) {
      showToast(errMessage(e) || "Não foi possível mudar de branch.", "err");
    }
  }

  el.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled) return;
    const repo = deps.repo();
    const name = btn.dataset.branch ?? "";
    switch (btn.dataset.action) {
      case "branches-recheck":
        void refreshBranches(repo.id, true);
        break;
      case "branches-reload":
        void refreshBranches(repo.id, false).then((ok) => {
          if (ok) void refreshBranches(repo.id, true);
        });
        break;
      case "branch-merge":
        deps.openMerge(name, defaultBranchOf(repo.id)?.name ?? null);
        break;
      case "branch-checkout":
        void checkout(btn, name);
        break;
      case "branch-push":
        deps.openPush(name);
        break;
    }
  });

  const off = onBranchesChange((id) => {
    if (!disposed && id === deps.repo().id) render();
  });

  const start = deps.repo();
  if (start.exists) {
    void refreshBranches(start.id, false).then((ok) => {
      const now = branchStore(start.id);
      if (ok && !disposed && (now.checkedAt === null || Date.now() - now.checkedAt > STALE_MS)) void refreshBranches(start.id, true);
    });
  }
  render();

  return {
    refresh: render,
    dispose: () => {
      disposed = true;
      off();
    },
  };
}
