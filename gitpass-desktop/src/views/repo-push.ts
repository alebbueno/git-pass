import "./repo-push.css";
import type { AccountRow } from "../lib/api";
import {
  reposApi,
  type CheckSeverity,
  type CommitFile,
  type OutgoingCommit,
  type OutgoingState,
  type PushCheck,
  type PushProgress,
  type PushResult,
  type RepoRow,
} from "../lib/api-repos";
import { errMessage, esc, relativeTime, showToast } from "../lib/dom";
import { icon, type IconName } from "../lib/icons";
import { profileChip } from "../lib/profile";
import { branchesOf, defaultBranchOf, ensureBranches, onBranchesChange } from "./repo-branches";
import { formatBytes, identityText, openModal, plural, profileFor, repoName } from "./repos-ui";

// ---------- Textos ----------

export type CheckCode = PushCheck["code"];

export const CHECK_TITLE: Record<CheckCode, string> = {
  author_mismatch: "Autor diferente da conta",
  committer_mismatch: "Committer diferente da conta",
  large_file: "Arquivo grande",
  file_too_large: "Arquivo grande demais para o GitHub",
  secret_suspected: "Possível senha ou chave nos arquivos",
  behind_remote: "Há commits novos no GitHub",
  no_push_permission: "A conta não tem permissão para enviar",
  account_disconnected: "Conta desconectada",
  detached_head: "Você não está em um branch",
  no_remote: "Sem endereço do GitHub",
  dirty_worktree: "Alterações sem commit",
  merge_commit: "Commit de merge",
  fetch_failed: "Não deu para verificar no GitHub",
  nothing_to_push: "Nada para enviar",
  merge_conflict: "Os dois branches mudaram as mesmas partes",
  target_dirty: "A pasta do destino tem alterações sem commit",
  target_behind_remote: "O GitHub tem commits novos no destino",
  already_in_target: "Alguns commits já estão no destino",
  source_pushed: "Esses commits já estão no GitHub",
};

/** Rótulo curto para os badges de cada commit. */
export const CHECK_SHORT: Record<CheckCode, string> = {
  author_mismatch: "Autor diferente",
  committer_mismatch: "Committer diferente",
  large_file: "Arquivo grande",
  file_too_large: "Arquivo grande demais",
  secret_suspected: "Possível segredo",
  behind_remote: "Atrás do GitHub",
  no_push_permission: "Sem permissão",
  account_disconnected: "Conta desconectada",
  detached_head: "Fora de um branch",
  no_remote: "Sem remote",
  dirty_worktree: "Sem commit",
  merge_commit: "Merge",
  fetch_failed: "Sem verificação",
  nothing_to_push: "Nada para enviar",
  merge_conflict: "Conflito",
  target_dirty: "Destino sem commit",
  target_behind_remote: "Destino atrás do GitHub",
  already_in_target: "Já no destino",
  source_pushed: "Já no GitHub",
};

export const CHECK_HINT: Partial<Record<CheckCode, string>> = {
  file_too_large: "O GitHub recusa arquivos com mais de 100 MB. Tire o arquivo do commit (ou use o Git LFS) antes de enviar.",
  large_file: "Arquivos com mais de 50 MB deixam o repositório pesado. Confira se precisa mesmo enviar.",
  secret_suspected: "Abra o arquivo e confira. Se for uma senha ou chave de verdade, tire do commit e troque a chave.",
  detached_head: "Volte para um branch no Terminal (por exemplo, git switch main) e clique em “Verificar de novo”.",
  no_remote: "Adicione o endereço do GitHub (git remote add origin …) e clique em “Verificar de novo”.",
  dirty_worktree: "Alterações sem commit não serão enviadas.",
  merge_commit: "O commit de merge vai junto com os outros.",
  merge_conflict: "Resolva no seu editor e volte aqui para ver a prévia de novo.",
  target_dirty: "Faça commit ou guarde (git stash) as alterações dessa pasta e veja a prévia de novo.",
  target_behind_remote: "É melhor trazer esses commits do GitHub para o destino antes. Assim você não cria diferenças que depois dão trabalho.",
  already_in_target: "Esses commits serão pulados. Os outros continuam.",
  source_pushed: "O branch de origem continua com eles. No destino eles entram como cópias, com códigos novos.",
};

export const SEVERITY_ORDER: CheckSeverity[] = ["error", "warning", "info"];

export const SEVERITY: Record<CheckSeverity, { title: string; icon: IconName; label: string }> = {
  error: { title: "Precisa resolver antes de enviar", icon: "x", label: "Bloqueia o envio" },
  warning: { title: "Confira antes de enviar", icon: "alert", label: "Aviso" },
  info: { title: "Para saber", icon: "info", label: "Informação" },
};

const PHASE_LABEL: Record<PushProgress["phase"], string> = {
  validating: "Conferindo os commits de novo…",
  connecting: "Conectando ao GitHub…",
  counting: "Preparando os arquivos…",
  compressing: "Compactando os arquivos…",
  writing: "Enviando para o GitHub…",
  resolving: "Finalizando no GitHub…",
  done: "Pronto",
};

/** Faixa [início, fim] de cada fase na barra (as fases do Git são sequenciais). */
const PHASE_SPAN: Record<PushProgress["phase"], [number, number]> = {
  validating: [0, 5],
  connecting: [5, 10],
  counting: [10, 20],
  compressing: [20, 35],
  writing: [35, 95],
  resolving: [95, 99],
  done: [100, 100],
};

const FILE_STATUS: Record<CommitFile["status"], string> = {
  added: "Novo",
  modified: "Alterado",
  deleted: "Removido",
  renamed: "Renomeado",
  copied: "Copiado",
  type_changed: "Tipo alterado",
};

// ---------- Sessões (uma por repositório, mantidas entre telas) ----------

type FilesEntry = { status: "loading" } | { status: "ok"; files: CommitFile[] } | { status: "error"; error: string };

type PushRun =
  | { phase: "idle" }
  | { phase: "running"; progress: PushProgress | null; count: number }
  | { phase: "done"; result: PushResult; target: string | null; accountId: string }
  | { phase: "failed"; error: string };

type Session = {
  id: string;
  /** Branch escolhido para enviar; `null` = o branch atual da pasta. */
  branch: string | null;
  state: OutgoingState | null;
  /** Hora (ms) do último estado vindo de um fetch. */
  checkedAt: number | null;
  error: string | null;
  checkError: string | null;
  pendingLocal: number;
  pendingFetch: number;
  seq: number;
  appliedSeq: number;
  ack: boolean;
  ackKey: string;
  expanded: Set<string>;
  files: Map<string, FilesEntry>;
  run: PushRun;
  action: "fix" | "pull" | null;
};

export type SessionEvent = "all" | "progress" | "pushed";
type Listener = (id: string, what: SessionEvent) => void;

const sessions = new Map<string, Session>();
const listeners = new Set<Listener>();

function emit(id: string, what: SessionEvent = "all"): void {
  for (const l of listeners) l(id, what);
}

export function onSessionChange(l: Listener): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

function session(id: string): Session {
  let s = sessions.get(id);
  if (!s) {
    s = {
      id,
      branch: null,
      state: null,
      checkedAt: null,
      error: null,
      checkError: null,
      pendingLocal: 0,
      pendingFetch: 0,
      seq: 0,
      appliedSeq: 0,
      ack: false,
      ackKey: "",
      expanded: new Set(),
      files: new Map(),
      run: { phase: "idle" },
      action: null,
    };
    sessions.set(id, s);
  }
  return s;
}

/** Commits ainda não enviados do branch atual (SHA completos), quando já carregados. */
export function outgoingShas(id: string): string[] | null {
  const s = sessions.get(id);
  if (!s || s.branch !== null) return null;
  return s.state?.commits.map((c) => c.sha) ?? null;
}

export function pushBranch(id: string): string | null {
  return sessions.get(id)?.branch ?? null;
}

/**
 * Escolhe o branch da aba Enviar (`null` = o atual). Descarta o estado do branch anterior;
 * quem chama decide quando carregar de novo (`openPushSession`). Não troca durante um envio.
 */
export function setPushBranch(id: string, branch: string | null): boolean {
  const s = session(id);
  if (s.branch === branch || s.run.phase === "running") return false;
  s.branch = branch;
  s.state = null;
  s.checkedAt = null;
  s.error = null;
  s.checkError = null;
  s.ack = false;
  s.ackKey = "";
  s.expanded.clear();
  s.run = { phase: "idle" };
  s.appliedSeq = ++s.seq;
  emit(id);
  return true;
}

export function forgetSession(id: string): void {
  sessions.delete(id);
}

export function isPushing(id: string): boolean {
  return sessions.get(id)?.run.phase === "running";
}

function ackKeyOf(st: OutgoingState): string {
  const warnings = st.checks
    .filter((c) => c.severity === "warning")
    .map((c) => `${c.code}:${c.sha ?? ""}:${c.path ?? ""}`)
    .sort();
  return `${st.head_sha ?? ""}|${warnings.join(",")}`;
}

function applyState(s: Session, st: OutgoingState): void {
  s.state = st;
  if (st.fetched) s.checkedAt = Date.now();
  const key = ackKeyOf(st);
  if (key !== s.ackKey) {
    s.ackKey = key;
    s.ack = false;
  }
  const shas = new Set(st.commits.map((c) => c.sha));
  for (const sha of s.expanded) if (!shas.has(sha)) s.expanded.delete(sha);
}

export async function refreshOutgoing(id: string, fetch: boolean): Promise<boolean> {
  const s = session(id);
  const seq = ++s.seq;
  const branch = s.branch;
  if (fetch) s.pendingFetch++;
  else s.pendingLocal++;
  emit(id);
  let ok = false;
  try {
    const st = await reposApi.outgoing(id, fetch, branch);
    if (seq > s.appliedSeq && s.branch === branch) {
      s.appliedSeq = seq;
      applyState(s, st);
      s.error = null;
      if (fetch) s.checkError = null;
    }
    ok = true;
  } catch (e) {
    if (seq > s.appliedSeq && s.branch === branch) {
      if (fetch && s.state) s.checkError = errMessage(e);
      else s.error = errMessage(e);
    }
  } finally {
    if (fetch) s.pendingFetch--;
    else s.pendingLocal--;
    emit(id);
  }
  return ok;
}

/** Ao abrir o repositório: primeiro o estado local (rápido), depois a verificação no GitHub. */
export function openPushSession(id: string): void {
  const s = session(id);
  if (s.run.phase === "running") return;
  if (s.run.phase !== "idle") s.run = { phase: "idle" };
  void refreshOutgoing(id, false).then((ok) => {
    if (ok && session(id).run.phase !== "running") void refreshOutgoing(id, true);
  });
}

let progressReady: Promise<void> | null = null;

function ensureProgressListener(): Promise<void> {
  if (!progressReady) {
    progressReady = reposApi
      .onPushProgress((p) => {
        const s = sessions.get(p.binding_id);
        if (!s || s.run.phase !== "running") return;
        s.run.progress = p;
        emit(p.binding_id, "progress");
      })
      .then(() => undefined)
      .catch(() => {
        // Sem eventos, a barra fica na primeira fase; o envio continua.
        progressReady = null;
      });
  }
  return progressReady;
}

function overallPercent(p: PushProgress | null): number {
  if (!p) return 0;
  const [a, b] = PHASE_SPAN[p.phase] ?? [0, 0];
  const pct = p.percent === null ? 0 : Math.max(0, Math.min(100, p.percent));
  return Math.round(a + ((b - a) * pct) / 100);
}

async function runPush(repo: RepoRow, st: OutgoingState, ack: boolean): Promise<void> {
  const s = session(repo.id);
  if (s.run.phase === "running" || !st.head_sha) return;
  const target = targetOf(st);
  s.run = { phase: "running", progress: null, count: st.ahead };
  emit(repo.id);
  await ensureProgressListener();
  try {
    const result = await reposApi.push(repo.id, st.head_sha, ack, s.branch);
    s.run = { phase: "done", result, target, accountId: repo.account_id };
    s.ack = false;
  } catch (e) {
    s.run = { phase: "failed", error: errMessage(e) || "Algo deu errado." };
  }
  emit(repo.id);
  emit(repo.id, "pushed");
  void refreshOutgoing(repo.id, false);
}

function targetOf(st: OutgoingState): string | null {
  if (st.upstream) return st.upstream;
  if (st.remote_name && st.branch) return `${st.remote_name}/${st.branch}`;
  return null;
}

// ---------- Validações (partilhadas com «Levar commits para outro branch») ----------

export type CheckGroupsOptions = {
  commits: OutgoingCommit[];
  /** SHA vira botão `goto-commit` (tratado por quem mostra a lista de commits). */
  linkCommits?: boolean;
  /** Títulos de cada grupo; o padrão fala de envio. */
  titles?: Partial<Record<CheckSeverity, string>>;
  /** HTML extra de cada tipo de check (botão ou dica). */
  action?: (code: CheckCode) => string;
};

/** Checks agrupados por gravidade, com ícone e texto (o mesmo visual da aba Enviar). */
export function checkGroupsHtml(checks: PushCheck[], o: CheckGroupsOptions): string {
  const groups = SEVERITY_ORDER.map((sev) => {
    const list = checks.filter((c) => c.severity === sev);
    if (list.length === 0) return "";
    const title = o.titles?.[sev] ?? SEVERITY[sev].title;
    const byCode = new Map<CheckCode, PushCheck[]>();
    for (const c of list) {
      const arr = byCode.get(c.code) ?? [];
      arr.push(c);
      byCode.set(c.code, arr);
    }
    const items = [...byCode.entries()]
      .map(([code, group]) => {
        const seen = new Set<string>();
        const msgs = group
          .filter((c) => {
            const k = `${c.message}|${c.sha ?? ""}|${c.path ?? ""}`;
            if (seen.has(k)) return false;
            seen.add(k);
            return true;
          })
          .map((c) => {
            const commit = c.sha ? o.commits.find((x) => x.sha === c.sha || x.sha.startsWith(c.sha!)) : undefined;
            const shaBtn =
              commit && o.linkCommits
                ? ` <button type="button" class="push-sha-link mono" data-action="goto-commit" data-sha="${esc(commit.sha)}" data-focus="goto-${esc(
                    commit.short_sha
                  )}-${esc(code)}" aria-label="Ver o commit ${esc(commit.short_sha)}">${esc(commit.short_sha)}</button>`
                : c.sha
                  ? ` <span class="mono muted">${esc(commit?.short_sha ?? c.sha.slice(0, 7))}</span>`
                  : "";
            const path = c.path ? ` <code class="push-path">${esc(c.path)}</code>` : "";
            return `<li>${esc(c.message)}${path}${shaBtn}</li>`;
          })
          .join("");
        const action = o.action ? o.action(code) : "";
        return `<li class="push-check push-check-${sev}">
          <span class="push-check-icon" aria-hidden="true">${icon(SEVERITY[sev].icon, 14)}</span>
          <div class="push-check-body">
            <p class="push-check-title"><span class="sr-only">${esc(SEVERITY[sev].label)}: </span>${esc(CHECK_TITLE[code] ?? code)}</p>
            <ul class="push-check-msgs small">${msgs}</ul>
            ${action ? `<div class="push-inline-actions">${action}</div>` : ""}
          </div>
        </li>`;
      })
      .join("");
    return `<section class="push-group push-group-${sev}" aria-label="${esc(title)}">
      <h4 class="push-group-title">${icon(SEVERITY[sev].icon, 14)}<span>${esc(title)}</span><span class="badge ${
        sev === "info" ? "" : sev
      }">${list.length}</span></h4>
      <ul class="push-checks">${items}</ul>
    </section>`;
  }).join("");
  return `<div class="push-validation" aria-label="Validação">${groups}</div>`;
}

// ---------- Aba «Enviar» ----------

export type PushTabDeps = {
  repo: () => RepoRow;
  accounts: () => AccountRow[];
  goAccounts: () => void;
  changeAccount: (btn: HTMLButtonElement) => void;
  fixIdentity: (btn: HTMLButtonElement) => void;
  /** Abre «Levar commits para outro branch». */
  openMerge: (source: string, target: string) => void;
  /** O branch escolhido mudou (para atualizar a rota). */
  onBranchChange: () => void;
};

export function mountPushTab(el: HTMLElement, deps: PushTabDeps): { refresh: () => void; dispose: () => void } {
  let pendingFocus: string | null = null;
  let lastPhaseText = "";
  let disposed = false;
  /** O conteúdo é refeito a cada mudança; os anúncios vão para uma região fixa para o leitor de tela não perdê-los. */
  el.innerHTML = `<p class="sr-only" role="status" aria-live="polite" data-ref="live"></p><div class="push-root"></div>`;
  const live = el.querySelector<HTMLElement>('[data-ref="live"]')!;
  const body = el.querySelector<HTMLElement>(".push-root")!;
  let prev: { checking: boolean; run: PushRun["phase"]; action: Session["action"] } | null = null;

  const cur = (): { repo: RepoRow; s: Session } => {
    const repo = deps.repo();
    return { repo, s: session(repo.id) };
  };

  /** O branch enviado é o da pasta: só aí valem as ações que mexem na pasta. */
  const onCurrent = (repo: RepoRow, s: Session): boolean => s.branch === null || s.branch === repo.branch;

  function announce(text: string): void {
    live.textContent = "";
    window.setTimeout(() => {
      live.textContent = text;
    }, 30);
  }

  function announceTransitions(s: Session): void {
    const now = { checking: s.pendingFetch > 0, run: s.run.phase, action: s.action };
    const before = prev;
    prev = now;
    if (!before) return;
    if (now.run !== before.run) {
      if (now.run === "running") announce("Enviando os commits…");
      else if (s.run.phase === "done") announce(`Pronto: ${plural(s.run.result.pushed, "commit enviado", "commits enviados")}.`);
      else if (s.run.phase === "failed") announce(`Não foi possível enviar: ${s.run.error}`);
      return;
    }
    if (before.checking && !now.checking && s.state) {
      announce(
        s.checkError
          ? "Não deu para verificar no GitHub. Mostrando o que está neste Mac."
          : s.state.ahead > 0
            ? `Verificado no GitHub. ${plural(s.state.ahead, "commit pronto", "commits prontos")} para enviar.`
            : "Verificado no GitHub. Tudo enviado."
      );
    } else if (!before.checking && now.checking) announce("Verificando no GitHub…");
  }

  function render(): void {
    const { repo, s } = cur();
    const active = document.activeElement as HTMLElement | null;
    const focusKey = active && body.contains(active) ? active.dataset.focus : undefined;
    body.innerHTML = html(repo, s);
    lastPhaseText = body.querySelector<HTMLElement>(".push-progress-phase")?.textContent ?? "";
    announceTransitions(s);
    const key = pendingFocus ?? focusKey;
    pendingFocus = null;
    if (key) body.querySelector<HTMLElement>(`[data-focus="${CSS.escape(key)}"]`)?.focus({ preventScroll: !!focusKey });
  }

  function renderProgress(): void {
    const { s } = cur();
    if (s.run.phase !== "running") return;
    const box = body.querySelector<HTMLElement>(".push-progress");
    if (!box) {
      render();
      return;
    }
    const pct = overallPercent(s.run.progress);
    box.querySelector<HTMLElement>(".push-progress-track")?.setAttribute("aria-valuenow", String(pct));
    const fill = box.querySelector<HTMLElement>(".push-progress-fill");
    if (fill) fill.style.width = `${pct}%`;
    const pctEl = box.querySelector<HTMLElement>(".push-progress-pct");
    if (pctEl) pctEl.textContent = `${pct}%`;
    const text = s.run.progress ? PHASE_LABEL[s.run.progress.phase] : PHASE_LABEL.validating;
    const phase = box.querySelector<HTMLElement>(".push-progress-phase");
    if (phase && text !== lastPhaseText) {
      phase.textContent = text;
      lastPhaseText = text;
      announce(text);
    }
  }

  // ---------- HTML ----------

  function html(repo: RepoRow, s: Session): string {
    if (!repo.exists) {
      return `<div class="callout error" role="alert">${icon("alert")}<span>A pasta não foi encontrada neste Mac. Não dá para conferir nem enviar commits. Se você moveu a pasta, desvincule o repositório e vincule de novo no lugar novo.</span></div>`;
    }
    if (s.run.phase === "done") return successHtml(repo, s.run);
    if (!s.state) {
      if (s.error) {
        return `<div class="push">${branchBarHtml(repo, s)}<div class="callout error push-callout-action" role="alert">${icon(
          "alert"
        )}<span>Não foi possível ler os commits deste repositório: ${esc(s.error)}</span>
          <button type="button" class="btn" data-action="reload" data-focus="reload">${icon("refresh", 14)}<span>Tentar de novo</span></button></div></div>`;
      }
      return `<div class="push">${branchBarHtml(repo, s)}<div class="push-loading" aria-busy="true"><span class="sr-only">Lendo os commits…</span>
        <div class="skeleton push-skeleton-title"></div>
        ${Array.from({ length: 4 }, () => `<div class="skeleton push-skeleton-row"></div>`).join("")}
      </div></div>`;
    }
    const st = s.state;
    return `
      <div class="push">
        ${branchBarHtml(repo, s)}
        ${statusLineHtml(s)}
        ${s.run.phase === "failed" ? failedHtml(s.run.error) : ""}
        ${summaryHtml(repo, st)}
        ${wrongBranchHtml(repo, st, s)}
        ${onCurrent(repo, s) ? identityHintHtml(repo) : ""}
        ${checksHtml(st, s)}
        ${st.commits.length ? commitsHtml(st, s) : ""}
        ${st.ahead > 0 || s.run.phase === "running" ? barHtml(repo, st, s) : ""}
      </div>`;
  }

  function statusLineHtml(s: Session): string {
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
    const busy = checking || s.run.phase === "running" || s.action !== null;
    return `<div class="push-status${s.checkError ? " warn" : ""}">
      <p class="push-status-text small">${text}</p>
      <button type="button" class="btn ghost" data-action="recheck" data-focus="recheck"${busy ? " disabled" : ""}>${icon(
        "refresh",
        14
      )}<span>Verificar de novo</span></button>
    </div>`;
  }

  function branchBarHtml(repo: RepoRow, s: Session): string {
    const chosen = s.branch ?? repo.branch;
    const local = (branchesOf(repo.id) ?? []).filter((b) => !b.remote_only);
    const names = local.map((b) => b.name);
    if (chosen && !names.includes(chosen)) names.unshift(chosen);
    const notCurrent = !onCurrent(repo, s);
    const note = notCurrent
      ? `<span class="badge push-branch-note">${icon("info", 12)} Não é o branch da pasta: ela não muda</span>`
      : "";
    if (names.length <= 1) {
      return `<div class="push-branch-bar">
        <p class="push-branch-label small">${icon("branch", 14)}<span>Enviando o branch:</span> ${
          chosen ? `<code>${esc(chosen)}</code>` : `<span class="muted">fora de um branch</span>`
        }</p>${note}
      </div>`;
    }
    const busy = s.run.phase === "running" || s.action !== null;
    const options = names
      .map((name) => {
        const info = local.find((b) => b.name === name);
        const extra = [name === repo.branch ? "atual" : "", info?.ahead ? `${info.ahead.toLocaleString("pt-BR")} para enviar` : ""]
          .filter(Boolean)
          .join(" · ");
        return `<option value="${esc(name)}"${name === chosen ? " selected" : ""}>${esc(extra ? `${name} (${extra})` : name)}</option>`;
      })
      .join("");
    return `<div class="push-branch-bar">
      <label class="push-branch-label small" for="push-branch-select">${icon("branch", 14)}<span>Enviando o branch:</span></label>
      <select class="select push-branch-select mono" id="push-branch-select" data-action="pick-branch" data-focus="pick-branch"${
        busy ? " disabled" : ""
      }>${repo.branch ? "" : `<option value=""${chosen ? "" : " selected"}>fora de um branch</option>`}${options}</select>
      ${note}
    </div>`;
  }

  function wrongBranchHtml(repo: RepoRow, st: OutgoingState, s: Session): string {
    const def = defaultBranchOf(repo.id);
    if (!def || !st.branch || st.branch === def.name || st.ahead === 0 || s.run.phase === "running") return "";
    const lead = onCurrent(repo, s) ? "Você está no branch" : "Você está enviando o branch";
    return `<div class="callout push-wrong-branch push-callout-action">${icon("branch")}<span>${lead} <code>${esc(
      st.branch
    )}</code>. Esses commits deveriam ir para <code>${esc(def.name)}</code>?</span>
      <button type="button" class="btn" data-action="merge-to-default" data-branch="${esc(st.branch)}" data-target="${esc(
        def.name
      )}" data-focus="merge-to-default">${icon("merge", 14)}<span>${esc(`Levar para ${def.name}…`)}</span></button></div>`;
  }

  function failedHtml(error: string): string {
    return `<div class="callout error push-failed" role="alert">${icon("alert")}<div>
      <p class="push-failed-title"><strong>Não foi possível enviar os commits.</strong></p>
      <p class="push-failed-msg">${esc(error)}</p>
      <div class="push-inline-actions">
        <button type="button" class="btn" data-action="push" data-focus="retry-push">${icon("refresh", 14)}<span>Tentar de novo</span></button>
        <button type="button" class="btn ghost" data-action="recheck" data-focus="failed-recheck">${icon("search", 14)}<span>Verificar de novo</span></button>
      </div>
    </div></div>`;
  }

  function accountChip(repo: RepoRow): string {
    if (repo.account_missing) return `<span class="badge error">${icon("x", 12)}conta removida</span>`;
    return profileChip(profileFor(deps.accounts(), repo));
  }

  function summaryHtml(repo: RepoRow, st: OutgoingState): string {
    const target = targetOf(st);
    if (st.ahead === 0) {
      const sub = !st.branch
        ? "Você não está em nenhum branch. Volte para um branch para conferir e enviar commits."
        : st.behind > 0
          ? `O GitHub tem ${plural(st.behind, "commit novo", "commits novos")} que ainda não ${st.behind === 1 ? "está" : "estão"} neste Mac.`
          : "Não há commits novos neste branch. Crie commits no seu editor ou no Terminal e volte aqui para conferir e enviar.";
      return `<div class="push-empty">
        <div class="empty-icon" aria-hidden="true">${icon(st.branch ? "check" : "branch", 24)}</div>
        <h3 class="push-empty-title" tabindex="-1">${!st.branch ? "Fora de um branch" : st.behind > 0 ? "Nada para enviar" : "Tudo enviado"}</h3>
        <p class="push-empty-sub">${esc(sub)}</p>
      </div>`;
    }
    const where = target ? ` para <code>${esc(target)}</code>` : "";
    return `<div class="push-summary">
      <h3 class="push-summary-title" tabindex="-1">${esc(plural(st.ahead, "commit pronto", "commits prontos"))} para enviar</h3>
      <p class="push-summary-sub">${where ? `${where.trim()} ` : ""}com ${accountChip(repo)}</p>
      ${
        !st.upstream && st.branch
          ? `<p class="callout push-new-branch">${icon("branch", 14)}<span>Este branch ainda não existe no GitHub — será criado${
              target ? ` como <code>${esc(target)}</code>` : ""
            }.</span></p>`
          : ""
      }
      ${st.truncated ? `<p class="muted small push-truncated">Mostrando os ${st.commits.length} commits mais recentes.</p>` : ""}
    </div>`;
  }

  function identityHintHtml(repo: RepoRow): string {
    const state = repo.identity.state;
    if (!state || state === "correct" || repo.account_missing) return "";
    const expected = identityText(repo.identity.expected_name, repo.identity.expected_email);
    return `<div class="callout warning push-callout-action">${icon("alert")}<span>${
      state === "different"
        ? "A identidade do Git nesta pasta é diferente da conta."
        : "Esta pasta não tem identidade do Git definida."
    } Os próximos commits podem sair com outro autor. A da conta é <span class="mono">${esc(expected)}</span>.</span>
      <button type="button" class="btn" data-action="fix-identity" data-focus="fix-identity">${icon("check", 14)}<span>Corrigir identidade</span></button></div>`;
  }

  function dirtyText(st: OutgoingState): string {
    const d = st.dirty;
    const parts: string[] = [];
    if (d.staged) parts.push(plural(d.staged, "preparada para commit", "preparadas para commit"));
    if (d.unstaged) parts.push(plural(d.unstaged, "modificada", "modificadas"));
    if (d.untracked) parts.push(plural(d.untracked, "nova", "novas"));
    return parts.length ? ` (${parts.join(", ")})` : "";
  }

  function isDirty(st: OutgoingState): boolean {
    return st.dirty.staged + st.dirty.unstaged + st.dirty.untracked > 0;
  }

  function fixReason(st: OutgoingState): string {
    if (st.commits.some((c) => c.is_merge)) return "Não dá para corrigir aqui porque há commits de merge entre os que vão ser enviados.";
    if (isDirty(st)) return "Faça commit ou guarde (git stash) as alterações sem commit antes de corrigir.";
    return "Não dá para corrigir automaticamente agora. Clique em “Verificar de novo”.";
  }

  function pullReason(st: OutgoingState): string {
    if (!st.upstream) return "";
    if (isDirty(st)) return "Faça commit ou guarde (git stash) as alterações sem commit antes de trazer do GitHub.";
    return "";
  }

  function checkActionHtml(code: CheckCode, st: OutgoingState, s: Session, fixShown: { v: boolean }): string {
    const busy = s.action !== null || s.run.phase === "running" || s.pendingFetch > 0;
    const spin = (label: string): string => `<span class="spinner" aria-hidden="true"></span><span>${esc(label)}</span>`;
    const current = onCurrent(deps.repo(), s);
    switch (code) {
      case "author_mismatch":
      case "committer_mismatch": {
        if (fixShown.v) return "";
        fixShown.v = true;
        if (!current && !st.can_fix_authors)
          return `<span class="small push-reason">Para corrigir o autor, mude a pasta para este branch (aba Branches) e volte aqui.</span>`;
        const reasonId = `push-fix-reason-${code}`;
        return st.can_fix_authors
          ? `<button type="button" class="btn" data-action="fix-authors" data-focus="fix-authors"${busy ? " disabled" : ""}${
              s.action === "fix" ? ' aria-busy="true"' : ""
            }>${s.action === "fix" ? spin("Corrigindo…") : `${icon("edit", 14)}<span>Corrigir autor dos commits</span>`}</button>`
          : `<button type="button" class="btn" disabled aria-describedby="${reasonId}">${icon("edit", 14)}<span>Corrigir autor dos commits</span></button>
             <span class="small push-reason" id="${reasonId}">${esc(fixReason(st))}</span>`;
      }
      case "behind_remote": {
        if (st.can_pull) {
          return `<button type="button" class="btn" data-action="pull" data-focus="pull"${busy ? " disabled" : ""}${
            s.action === "pull" ? ' aria-busy="true"' : ""
          }>${s.action === "pull" ? spin("Trazendo…") : `${icon("arrowDown", 14)}<span>Trazer do GitHub</span>`}</button>`;
        }
        if (!current)
          return `<span class="small push-reason">Para trazer do GitHub, mude a pasta para este branch (aba Branches) e volte aqui.</span>`;
        const why = pullReason(st);
        return why ? `<span class="small push-reason">${esc(why)}</span>` : "";
      }
      case "account_disconnected":
        return `<button type="button" class="btn" data-action="reconnect" data-focus="reconnect">${icon("user", 14)}<span>Reconectar conta</span></button>`;
      case "no_push_permission":
        return `<span class="repos-menu-host menu-left"><button type="button" class="btn" data-action="change-account" data-focus="perm-change" aria-haspopup="menu" aria-expanded="false">${icon(
          "user",
          14
        )}<span>Trocar conta</span></button></span>`;
      case "fetch_failed":
        return `<button type="button" class="btn" data-action="recheck" data-focus="fetch-recheck"${busy ? " disabled" : ""}>${icon(
          "refresh",
          14
        )}<span>Verificar de novo</span></button>`;
      case "dirty_worktree":
        return `<span class="small push-reason">${esc(`${CHECK_HINT.dirty_worktree}${dirtyText(st)}`)}</span>`;
      default: {
        const hint = CHECK_HINT[code];
        return hint ? `<span class="small push-reason">${esc(hint)}</span>` : "";
      }
    }
  }

  function checksHtml(st: OutgoingState, s: Session): string {
    const shown = st.checks.filter((c) => c.code !== "nothing_to_push");
    if (shown.length === 0) {
      if (st.ahead === 0) return "";
      return `<div class="callout success push-allgood">${icon("check")}<span><strong>Tudo certo.</strong> Os commits estão com o autor certo e nada impede o envio.</span></div>`;
    }
    const fixShown = { v: false };
    return checkGroupsHtml(shown, {
      commits: st.commits,
      linkCommits: true,
      action: (code) => checkActionHtml(code, st, s, fixShown),
    });
  }

  function worst(checks: PushCheck[]): CheckSeverity | null {
    for (const sev of SEVERITY_ORDER) if (checks.some((c) => c.severity === sev)) return sev;
    return null;
  }

  function commitsHtml(st: OutgoingState, s: Session): string {
    const items = st.commits.map((c) => commitHtml(c, s)).join("");
    return `<section class="push-commits-section" aria-labelledby="push-commits-title">
      <h4 class="section-label push-commits-label" id="push-commits-title">${icon("commit", 14)} ${
        st.ahead > 0 ? "Commits para enviar" : "Commits"
      } <span class="push-commits-count">(${st.commits.length.toLocaleString("pt-BR")}${st.truncated ? "+" : ""})</span></h4>
      <ol class="push-commits">${items}</ol>
    </section>`;
  }

  function commitHtml(c: OutgoingCommit, s: Session): string {
    const open = s.expanded.has(c.sha);
    const sev = worst(c.checks);
    const authorBad = c.checks.some((k) => k.code === "author_mismatch");
    const codes = [...new Map(c.checks.map((k) => [k.code, k])).values()];
    const badges = codes
      .map((k) => `<span class="badge ${k.severity === "info" ? "" : k.severity}">${esc(CHECK_SHORT[k.code] ?? k.code)}</span>`)
      .join("");
    const panelId = `push-c-${c.sha}`;
    return `<li class="push-commit${sev ? ` push-commit-${sev}` : ""}${open ? " open" : ""}" data-sha="${esc(c.sha)}">
      <button type="button" class="push-commit-toggle" data-action="toggle-commit" data-sha="${esc(c.sha)}" data-focus="c-${esc(
        c.sha
      )}" aria-expanded="${open}" aria-controls="${esc(panelId)}">
        <span class="push-commit-chevron" aria-hidden="true">${icon("chevronRight", 14)}</span>
        <span class="push-commit-main">
          <span class="push-commit-subject">${c.is_merge ? `<span class="badge">Merge</span> ` : ""}${esc(c.subject || "(sem mensagem)")}</span>
          <span class="push-commit-meta small">
            <span class="mono push-sha">${esc(c.short_sha)}</span>
            <span class="push-author${authorBad ? " mismatch" : ""}" title="${esc(`${c.author_name} <${c.author_email}>`)}">${
              authorBad ? `${icon("alert", 12)}<span class="sr-only">Autor diferente da conta: </span>` : ""
            }${esc(c.author_name || c.author_email)}</span>
            <time datetime="${esc(c.authored_at)}" title="${esc(fullDate(c.authored_at))}">${esc(relativeTime(c.authored_at))}</time>
            <span class="push-stat"><span class="push-ins">+${c.insertions.toLocaleString("pt-BR")}</span> <span class="push-del">−${c.deletions.toLocaleString(
              "pt-BR"
            )}</span><span class="sr-only"> linhas em ${plural(c.files_changed, "arquivo", "arquivos")}</span></span>
          </span>
        </span>
        ${badges ? `<span class="push-commit-badges">${badges}</span>` : ""}
      </button>
      <div class="push-commit-detail" id="${esc(panelId)}"${open ? "" : " hidden"}>${open ? commitDetailHtml(c, s) : ""}</div>
    </li>`;
  }

  function fullDate(iso: string): string {
    const t = Date.parse(iso);
    return Number.isNaN(t) ? iso : new Date(t).toLocaleString("pt-BR", { dateStyle: "long", timeStyle: "short" });
  }

  function commitDetailHtml(c: OutgoingCommit, s: Session): string {
    const authorBad = c.checks.some((k) => k.code === "author_mismatch");
    const committerBad = c.checks.some((k) => k.code === "committer_mismatch");
    const sameCommitter = c.committer_email.toLowerCase() === c.author_email.toLowerCase() && c.committer_name === c.author_name;
    const body = c.body.trim();
    return `
      ${body ? `<pre class="push-commit-body">${esc(body)}</pre>` : ""}
      <dl class="push-commit-people small">
        <dt>Autor</dt><dd class="mono${authorBad ? " push-mismatch" : ""}">${authorBad ? `${icon("alert", 12)} ` : ""}${esc(
          `${c.author_name} <${c.author_email}>`
        )}</dd>
        ${
          sameCommitter
            ? ""
            : `<dt>Committer</dt><dd class="mono${committerBad ? " push-mismatch" : ""}">${committerBad ? `${icon("alert", 12)} ` : ""}${esc(
                `${c.committer_name} <${c.committer_email}>`
              )}</dd>`
        }
        <dt>Data</dt><dd>${esc(fullDate(c.authored_at))}</dd>
        <dt>Commit</dt><dd class="mono">${esc(c.sha)}</dd>
      </dl>
      <h5 class="push-files-title small">${icon("file", 14)} ${esc(plural(loadedFiles(c, s) ?? c.files_changed, "arquivo alterado", "arquivos alterados"))}</h5>
      ${filesHtml(c, s)}`;
  }

  function loadedFiles(c: OutgoingCommit, s: Session): number | null {
    const entry = s.files.get(c.sha);
    return entry?.status === "ok" ? entry.files.length : null;
  }

  function filesHtml(c: OutgoingCommit, s: Session): string {
    const entry = s.files.get(c.sha);
    if (!entry || entry.status === "loading") {
      return `<div class="push-files-loading" aria-busy="true"><span class="sr-only">Carregando os arquivos…</span>${Array.from(
        { length: Math.min(3, Math.max(1, c.files_changed)) },
        () => `<div class="skeleton push-skeleton-file"></div>`
      ).join("")}</div>`;
    }
    if (entry.status === "error") {
      return `<div class="callout error push-callout-action" role="alert">${icon("alert")}<span>Não foi possível listar os arquivos: ${esc(entry.error)}</span>
        <button type="button" class="btn" data-action="retry-files" data-sha="${esc(c.sha)}" data-focus="files-${esc(c.sha)}">${icon(
          "refresh",
          14
        )}<span>Tentar de novo</span></button></div>`;
    }
    if (entry.files.length === 0) return `<p class="muted small">Nenhum arquivo alterado neste commit.</p>`;
    const flagged = new Map<string, PushCheck>();
    for (const k of c.checks) if (k.path) flagged.set(k.path, k);
    return `<ul class="push-files">${entry.files
      .map((f) => {
        const flag = flagged.get(f.path);
        const stat = f.binary
          ? `<span class="muted">Binário</span>`
          : f.insertions === null && f.deletions === null
            ? ""
            : `<span class="push-ins">+${(f.insertions ?? 0).toLocaleString("pt-BR")}</span> <span class="push-del">−${(f.deletions ?? 0).toLocaleString(
                "pt-BR"
              )}</span>`;
        const path =
          f.old_path && f.old_path !== f.path
            ? `<span class="mono push-file-path" title="${esc(`${f.old_path} → ${f.path}`)}">${esc(f.old_path)} <span aria-hidden="true">→</span><span class="sr-only"> para </span> ${esc(
                f.path
              )}</span>`
            : `<span class="mono push-file-path" title="${esc(f.path)}">${esc(f.path)}</span>`;
        return `<li class="push-file">
          <span class="push-file-status push-file-${f.status}">${esc(FILE_STATUS[f.status] ?? f.status)}</span>
          ${path}
          ${flag ? `<span class="badge ${flag.severity === "info" ? "" : flag.severity}">${esc(CHECK_SHORT[flag.code] ?? flag.code)}</span>` : ""}
          <span class="push-file-stat small">${stat}</span>
          <span class="push-file-size small muted">${f.size_bytes === null ? "" : esc(formatBytes(f.size_bytes))}</span>
        </li>`;
      })
      .join("")}</ul>`;
  }

  function blockReason(st: OutgoingState, s: Session): string | null {
    if (!st.can_push) return "Resolva os itens em “Precisa resolver antes de enviar” para liberar o envio.";
    if (!st.head_sha) return "Não foi possível ler o commit atual. Clique em “Verificar de novo”.";
    if (s.pendingFetch > 0) return "Aguarde a verificação no GitHub terminar.";
    if (s.action === "fix") return "Corrigindo o autor dos commits…";
    if (s.action === "pull") return "Trazendo os commits do GitHub…";
    if (st.has_warnings && !s.ack) return "Confira os avisos e marque a confirmação para enviar.";
    return null;
  }

  function barHtml(repo: RepoRow, st: OutgoingState, s: Session): string {
    if (s.run.phase === "running") {
      const pct = overallPercent(s.run.progress);
      const phase = s.run.progress ? PHASE_LABEL[s.run.progress.phase] : PHASE_LABEL.validating;
      return `<div class="push-bar push-progress">
        <p class="push-progress-text"><span class="spinner" aria-hidden="true"></span><span class="push-progress-phase">${esc(
          phase
        )}</span></p>
        <div class="push-progress-row">
          <div class="push-progress-track" role="progressbar" aria-label="Enviando ${esc(plural(s.run.count, "commit", "commits"))} de ${esc(
            repoName(repo)
          )}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct}"><span class="push-progress-fill" style="width:${pct}%"></span></div>
          <span class="push-progress-pct small muted" aria-hidden="true">${pct}%</span>
        </div>
      </div>`;
    }
    const reason = blockReason(st, s);
    const ackBox =
      st.has_warnings && st.can_push
        ? `<label class="repos-check push-ack">
            <input type="checkbox" data-action="ack" data-focus="ack"${s.ack ? " checked" : ""}${s.action || s.pendingFetch > 0 ? " disabled" : ""} />
            <span>Revisei os avisos e quero enviar mesmo assim</span>
          </label>`
        : "";
    return `<div class="push-bar">
      ${ackBox}
      <div class="push-bar-row">
        <p class="push-bar-reason small${reason ? "" : " sr-only"}" id="push-bar-reason">${esc(reason ?? "Tudo pronto para enviar.")}</p>
        <button type="button" class="btn primary lg push-send" data-action="push" data-focus="send"${reason ? " disabled" : ""} aria-describedby="push-bar-reason">${icon(
          "send"
        )}<span>Enviar ${esc(plural(st.ahead, "commit", "commits"))}</span></button>
      </div>
    </div>`;
  }

  function successHtml(repo: RepoRow, run: Extract<PushRun, { phase: "done" }>): string {
    const r = run.result;
    const target = run.target ?? r.branch;
    const acc = deps.accounts().find((a) => a.id === run.accountId);
    const ghUrl = r.remote_branch_url ?? repo.remote_url;
    return `<div class="push-success">
      <div class="empty-icon push-success-icon" aria-hidden="true">${icon("check", 26)}</div>
      <h3 class="push-success-title" tabindex="-1" data-focus="success-title">${esc(
        r.pushed === 0 ? "Nada novo para enviar" : `${plural(r.pushed, "commit enviado", "commits enviados")}`
      )}</h3>
      <p class="push-success-sub">Para <code>${esc(target)}</code>${acc ? ` com ${profileChip(acc)}` : ""}</p>
      ${r.created_upstream ? `<p class="push-success-sub">${icon("branch", 14)} O branch foi criado no GitHub.</p>` : ""}
      <div class="push-success-actions">
        ${
          ghUrl
            ? `<a class="btn primary" href="${esc(ghUrl)}" data-focus="success-gh">${icon("external", 14)}<span>Ver no GitHub</span><span class="sr-only"> (abre no navegador)</span></a>`
            : ""
        }
        <button type="button" class="btn" data-action="done" data-focus="success-done">${icon("commit", 14)}<span>Voltar aos commits</span></button>
      </div>
      ${
        r.summary.trim()
          ? `<details class="push-success-details">
              <summary data-focus="success-details">Ver detalhes</summary>
              <pre class="push-success-log">${esc(r.summary)}</pre>
            </details>`
          : ""
      }
      <p class="sr-only">${esc(repoName(repo))}</p>
    </div>`;
  }

  // ---------- Ações ----------

  async function loadFiles(id: string, sha: string): Promise<void> {
    const s = session(id);
    s.files.set(sha, { status: "loading" });
    emit(id);
    try {
      const files = await reposApi.commitFiles(id, sha);
      s.files.set(sha, { status: "ok", files });
    } catch (e) {
      s.files.set(sha, { status: "error", error: errMessage(e) });
    }
    emit(id);
  }

  function toggleCommit(sha: string, forceOpen = false): void {
    const { repo, s } = cur();
    const open = forceOpen || !s.expanded.has(sha);
    if (open) s.expanded.add(sha);
    else s.expanded.delete(sha);
    pendingFocus = `c-${sha}`;
    render();
    if (open) {
      const entry = s.files.get(sha);
      if (!entry || entry.status === "error") void loadFiles(repo.id, sha);
    }
  }

  async function confirmPush(): Promise<void> {
    const { repo, s } = cur();
    const st = s.state;
    if (!st || !st.can_push || !st.head_sha || s.run.phase === "running") return;
    if (st.has_warnings && !s.ack) {
      pendingFocus = "ack";
      render();
      showToast("Confira os avisos e marque a confirmação para enviar.", "err");
      return;
    }
    const target = targetOf(st);
    const warnings = [...new Set(st.checks.filter((c) => c.severity === "warning").map((c) => CHECK_TITLE[c.code] ?? c.code))];
    const body = `
      <p class="repos-modal-lead">Os commits vão para o GitHub com a conta vinculada a este repositório.</p>
      <dl class="push-confirm">
        <dt>Repositório</dt><dd class="mono">${esc(repoName(repo))}</dd>
        <dt>Envio</dt><dd><code>${esc(st.branch ?? "branch atual")}</code> <span aria-hidden="true">→</span><span class="sr-only">para</span> <code>${esc(
          target ?? "GitHub"
        )}</code>${st.upstream ? "" : ` <span class="badge">novo no GitHub</span>`}</dd>
        <dt>Conta</dt><dd>${accountChip(repo)}</dd>
        <dt>Commits</dt><dd>${esc(st.ahead.toLocaleString("pt-BR"))}</dd>
        <dt>Avisos</dt><dd>${
          warnings.length
            ? `<ul class="push-confirm-warnings">${warnings.map((w) => `<li>${icon("alert", 12)}${esc(w)}</li>`).join("")}</ul>`
            : `<span class="ok">${icon("check", 12)}Nenhum</span>`
        }</dd>
      </dl>`;
    const ok = await openModal<boolean>({
      title: `Enviar ${plural(st.ahead, "commit", "commits")}?`,
      body,
      confirmLabel: `Enviar ${plural(st.ahead, "commit", "commits")}`,
      wide: true,
      read: () => true,
    });
    if (!ok) return;
    const now = session(repo.id).state;
    if (!now || now.head_sha !== st.head_sha) {
      showToast("Os commits mudaram enquanto você confirmava. Confira de novo antes de enviar.", "err");
      return;
    }
    await runPush(repo, st, st.has_warnings && s.ack);
    if (disposed || deps.repo().id !== repo.id) return;
    pendingFocus = session(repo.id).run.phase === "done" ? "success-title" : "retry-push";
    render();
  }

  async function fixAuthors(): Promise<void> {
    const { repo, s } = cur();
    const st = s.state;
    if (!st || !st.head_sha || !st.can_fix_authors || s.action || !onCurrent(repo, s)) return;
    const expected = identityText(repo.identity.expected_name, repo.identity.expected_email);
    const ok = await openModal<boolean>({
      title: "Corrigir o autor dos commits?",
      body: `
        <p class="repos-modal-lead">O GitPass vai reescrever ${esc(plural(st.ahead, "commit que ainda não foi enviado", "commits que ainda não foram enviados"))}, trocando o autor e o committer para:</p>
        <p class="mono push-modal-identity">${esc(expected)}</p>
        <ul class="push-modal-list small">
          <li>As mensagens e os arquivos não mudam.</li>
          <li>Os commits ganham códigos (SHA) novos.</li>
          <li>Commits que já estão no GitHub não são alterados.</li>
          <li>Se algo der errado, o branch volta a ficar como estava.</li>
        </ul>`,
      confirmLabel: `Corrigir ${plural(st.ahead, "commit", "commits")}`,
      read: () => true,
    });
    if (!ok) return;
    s.action = "fix";
    emit(repo.id);
    try {
      const next = await reposApi.fixAuthors(repo.id, st.head_sha);
      s.action = null;
      applyState(s, next);
      showToast(`Autor corrigido em ${plural(st.ahead, "commit", "commits")}.`, "ok");
    } catch (e) {
      s.action = null;
      showToast(errMessage(e) || "Não foi possível corrigir o autor dos commits.", "err");
      void refreshOutgoing(repo.id, false);
    }
    emit(repo.id);
  }

  async function pull(): Promise<void> {
    const { repo, s } = cur();
    const st = s.state;
    if (!st || !st.can_pull || s.action || !onCurrent(repo, s)) return;
    s.action = "pull";
    emit(repo.id);
    try {
      const next = await reposApi.pull(repo.id);
      s.action = null;
      applyState(s, next);
      showToast(`${plural(st.behind, "commit trazido", "commits trazidos")} do GitHub.`, "ok");
    } catch (e) {
      s.action = null;
      showToast(errMessage(e) || "Não foi possível trazer os commits do GitHub.", "err");
      void refreshOutgoing(repo.id, false);
    }
    emit(repo.id);
  }

  el.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled) return;
    const { repo, s } = cur();
    switch (btn.dataset.action) {
      case "reload":
        void refreshOutgoing(repo.id, false);
        break;
      case "recheck":
        if (s.run.phase === "failed") s.run = { phase: "idle" };
        pendingFocus = btn.dataset.focus ?? null;
        void refreshOutgoing(repo.id, true);
        break;
      case "push":
        void confirmPush();
        break;
      case "done":
        s.run = { phase: "idle" };
        render();
        body.querySelector<HTMLElement>(".push-summary-title, .push-empty-title")?.focus();
        break;
      case "toggle-commit":
        if (btn.dataset.sha) toggleCommit(btn.dataset.sha);
        break;
      case "goto-commit": {
        const sha = btn.dataset.sha;
        if (!sha) break;
        toggleCommit(sha, true);
        body.querySelector<HTMLElement>(`.push-commit[data-sha="${CSS.escape(sha)}"]`)?.scrollIntoView({ block: "nearest" });
        break;
      }
      case "retry-files":
        if (btn.dataset.sha) void loadFiles(repo.id, btn.dataset.sha);
        break;
      case "fix-authors":
        void fixAuthors();
        break;
      case "pull":
        void pull();
        break;
      case "reconnect":
        deps.goAccounts();
        break;
      case "change-account":
        deps.changeAccount(btn);
        break;
      case "fix-identity":
        deps.fixIdentity(btn);
        break;
      case "merge-to-default":
        if (btn.dataset.branch && btn.dataset.target) deps.openMerge(btn.dataset.branch, btn.dataset.target);
        break;
    }
  });

  el.addEventListener("change", (e) => {
    const input = e.target as HTMLInputElement | HTMLSelectElement;
    const { repo, s } = cur();
    if (input.dataset.action === "pick-branch") {
      const value = input.value || null;
      const next = value === repo.branch ? null : value;
      pendingFocus = "pick-branch";
      if (setPushBranch(repo.id, next)) {
        openPushSession(repo.id);
        deps.onBranchChange();
        announce(`Enviando o branch ${value ?? "atual"}.`);
      } else render();
      return;
    }
    if (input.dataset.action !== "ack" || !(input instanceof HTMLInputElement)) return;
    s.ack = input.checked;
    pendingFocus = "ack";
    render();
  });

  const off = onSessionChange((id, what) => {
    if (disposed || id !== deps.repo().id) return;
    if (what === "progress") renderProgress();
    else if (what === "all") render();
  });
  const branchSig = (id: string): string =>
    `${defaultBranchOf(id)?.name ?? ""}|${(branchesOf(id) ?? [])
      .filter((b) => !b.remote_only)
      .map((b) => `${b.name}:${b.ahead ?? ""}`)
      .join(",")}`;
  let lastBranchSig = branchSig(deps.repo().id);
  const offBranches = onBranchesChange((id) => {
    if (disposed || id !== deps.repo().id) return;
    const sig = branchSig(id);
    if (sig === lastBranchSig) return;
    lastBranchSig = sig;
    if (session(id).run.phase !== "running") render();
  });
  if (deps.repo().exists) ensureBranches(deps.repo().id);
  render();

  return {
    refresh: render,
    dispose: () => {
      disposed = true;
      off();
      offBranches();
    },
  };
}
