import "./repo-merge.css";
import {
  reposApi,
  type BranchInfo,
  type MergeMode,
  type MergePreview,
  type MergeResult,
  type OutgoingCommit,
  type RepoRow,
} from "../lib/api-repos";
import { errMessage, esc, relativeTime, shortenPath, showToast } from "../lib/dom";
import { icon } from "../lib/icons";
import { DATA_CHANGED_EVENT } from "../lib/view";
import { branchesOf, defaultBranchOf, ensureBranches, onBranchesChange, refreshBranches } from "./repo-branches";
import { CHECK_HINT, CHECK_TITLE, checkGroupsHtml } from "./repo-push";
import { plural, repoName } from "./repos-ui";

/** «Levar commits para outro branch»: escolher, ver a prévia (sem mexer em nada), confirmar e aplicar. */

export type MergeFlowOptions = {
  repo: () => RepoRow;
  source: string;
  target?: string | null;
  /** SHA (curto ou completo) que já vem marcado, no modo «só os commits que eu escolher». */
  commit?: string | null;
  /** «Enviar <destino> para o GitHub» depois de aplicar. */
  onPushTarget: (target: string) => void;
  onCheckedOut: (row: RepoRow) => void;
  onApplied: (result: MergeResult) => void;
};

type Step = 1 | 2 | 3 | 4;

const STEPS: { n: Step; label: string }[] = [
  { n: 1, label: "O quê" },
  { n: 2, label: "Prévia" },
  { n: 3, label: "Confirmar" },
  { n: 4, label: "Pronto" },
];

const DEBOUNCE_MS = 250;
const CONFIRM_LIST_MAX = 5;

let active: { focus: () => void } | null = null;
let seqId = 0;

export function openMergeFlow(opts: MergeFlowOptions): void {
  if (active) {
    active.focus();
    return;
  }
  const repoId = opts.repo().id;
  const uid = ++seqId;
  const opener = document.activeElement as HTMLElement | null;

  let step: Step = 1;
  let source = opts.source;
  let target: string | null = opts.target && opts.target !== opts.source ? opts.target : null;
  let mode: MergeMode = opts.commit ? "cherry_pick" : "merge";
  let modeTouched = !!opts.commit;
  /** `null` = ainda não definida para esta origem/destino (a primeira prévia decide o padrão). */
  let selected: Set<string> | null = null;
  let missingCommit: string | null = null;
  let preview: MergePreview | null = null;
  let previewKey = "";
  let previewError: string | null = null;
  let loading = false;
  let seq = 0;
  let timer: number | undefined;
  let ack = false;
  let ackKey = "";
  let applying = false;
  let applyError: string | null = null;
  let result: MergeResult | null = null;
  let checkingOut = false;
  let closed = false;
  let pendingFocus: string | null = null;
  let lastAnnounced = "";
  let techOpen = false;

  ensureBranches(repoId);

  // ---------- Dados ----------

  const localBranches = (): BranchInfo[] => (branchesOf(repoId) ?? []).filter((b) => !b.remote_only);
  const infoOf = (name: string | null): BranchInfo | undefined => (name ? (branchesOf(repoId) ?? []).find((b) => b.name === name) : undefined);

  function pickTarget(): string | null {
    if (target && target !== source) return target;
    const def = defaultBranchOf(repoId);
    if (def && def.name !== source && !def.remote_only) return def.name;
    return localBranches().find((b) => b.name !== source)?.name ?? null;
  }

  function orderedSelection(): string[] {
    if (!selected) return [];
    const list = preview?.commits ?? [];
    const ordered = list.filter((c) => selected!.has(c.sha)).map((c) => c.sha);
    for (const sha of selected) if (!ordered.includes(sha)) ordered.push(sha);
    return ordered.reverse();
  }

  const commitsArg = (): string[] => (mode === "cherry_pick" ? orderedSelection() : []);
  const requestKey = (): string => `${source}|${target ?? ""}|${mode}|${commitsArg().join(",")}`;
  const nothingPicked = (): boolean => mode === "cherry_pick" && selected !== null && selected.size === 0;
  const fresh = (): boolean => !!preview && previewKey === requestKey() && !loading && !previewError;

  function defaultSelection(p: MergePreview): Set<string> {
    missingCommit = null;
    const wanted = source === opts.source ? opts.commit?.toLowerCase() : null;
    if (wanted) {
      const c = p.commits.find((x) => x.sha.toLowerCase().startsWith(wanted));
      if (c) return new Set([c.sha]);
      missingCommit = wanted.slice(0, 7);
      return new Set();
    }
    const info = infoOf(source);
    const unpushed = info?.upstream && info.ahead !== null ? info.ahead : null;
    const pickable = (list: OutgoingCommit[]): Set<string> => new Set(list.filter((c) => !c.is_merge).map((c) => c.sha));
    if (unpushed !== null && unpushed > 0 && unpushed < p.commits.length) return pickable(p.commits.slice(0, unpushed));
    return pickable(p.commits);
  }

  /** A origem já tem commits no GitHub: levar só os escolhidos evita juntar o que já foi publicado. */
  function prefersPick(p: MergePreview): boolean {
    if (p.checks.some((c) => c.code === "source_pushed")) return true;
    const info = infoOf(source);
    return !!info?.upstream && info.ahead !== null && p.commits.length > info.ahead;
  }

  function schedulePreview(delay = DEBOUNCE_MS): void {
    window.clearTimeout(timer);
    target = pickTarget();
    if (!target || nothingPicked()) {
      seq++;
      loading = false;
      render();
      return;
    }
    loading = true;
    timer = window.setTimeout(() => void runPreview(), delay);
    render();
  }

  async function runPreview(): Promise<void> {
    if (!target) return;
    const mySeq = ++seq;
    const key = requestKey();
    const args = { source, target, mode, commits: commitsArg() };
    loading = true;
    render();
    try {
      const p = await reposApi.mergePreview(repoId, args.source, args.target, args.mode, args.commits);
      if (closed || mySeq !== seq) return;
      previewError = null;
      preview = p;
      previewKey = key;
      if (selected === null) {
        if (!modeTouched && prefersPick(p)) mode = "cherry_pick";
        selected = defaultSelection(p);
        if (requestKey() !== key) {
          schedulePreview(0);
          return;
        }
      } else {
        const shas = new Set(p.commits.map((c) => c.sha));
        for (const sha of [...selected]) if (!shas.has(sha)) selected.delete(sha);
      }
      const k = `${key}|${p.source_sha}|${p.target_sha}|${p.checks
        .filter((c) => c.severity === "warning")
        .map((c) => `${c.code}:${c.sha ?? ""}:${c.path ?? ""}`)
        .sort()
        .join(",")}`;
      if (k !== ackKey) {
        ackKey = k;
        ack = false;
      }
      loading = false;
      announcePreview(p);
    } catch (e) {
      if (closed || mySeq !== seq) return;
      loading = false;
      previewError = errMessage(e) || "Algo deu errado.";
      announce(`Não foi possível calcular a prévia: ${previewError}`);
    }
    render();
  }

  // ---------- Textos ----------

  const code = (s: string): string => `<code>${esc(s)}</code>`;

  function pickedCount(p: MergePreview): number {
    if (p.mode !== "cherry_pick") return p.commits.length;
    return p.selected.length || commitsArg().length;
  }

  function outcomeText(p: MergePreview): string {
    const t = code(p.target);
    if (p.conflicts.length) return `Os dois branches mudaram as mesmas partes de ${esc(plural(p.conflicts.length, "arquivo", "arquivos"))}.`;
    switch (p.strategy) {
      case "up_to_date":
        return `Esses commits já estão na ${t}.`;
      case "fast_forward":
        return `A ${t} só vai avançar até o último commit de ${code(p.source)}. Nenhum commit novo será criado.`;
      default: {
        if (p.mode === "merge") return `Será criado 1 commit de junção na ${t}, com tudo o que ${code(p.source)} tem e ela não tem.`;
        const n = pickedCount(p);
        return `${n === 1 ? "Será copiado 1 commit" : `Serão copiados ${n.toLocaleString("pt-BR")} commits`} para a ${t}. O branch ${code(
          p.source
        )} continua como está.`;
      }
    }
  }

  function whereText(p: MergePreview): string {
    if (!p.applies_in_worktree) return "Sua pasta não vai mudar.";
    const repo = opts.repo();
    if (p.target === repo.branch) return "Vai alterar a sua pasta (branch atual).";
    const wt = infoOf(p.target)?.worktree_path;
    return wt ? `Vai alterar a pasta onde a ${code(p.target)} está aberta: <span class="mono">${esc(shortenPath(wt))}</span>.` : "Vai alterar a pasta onde o destino está aberto.";
  }

  function applyLabel(): string {
    const t = target ?? "";
    if (mode === "merge") return `Juntar ${source} na ${t}`;
    const n = preview ? pickedCount(preview) : commitsArg().length;
    return `Levar ${plural(n, "commit", "commits")} para ${t}`;
  }

  function announce(text: string): void {
    if (text === lastAnnounced) return;
    lastAnnounced = text;
    live.textContent = "";
    window.setTimeout(() => {
      live.textContent = text;
    }, 30);
  }

  function announcePreview(p: MergePreview): void {
    const tmp = document.createElement("div");
    tmp.innerHTML = outcomeText(p);
    announce(`Prévia: ${tmp.textContent ?? ""}`);
  }

  // ---------- DOM ----------

  const titleId = `merge-title-${uid}`;
  const backdrop = document.createElement("div");
  backdrop.className = "modal-backdrop repos-modal-backdrop merge-backdrop";
  backdrop.innerHTML = `
    <div class="modal repos-modal merge-modal" role="dialog" aria-modal="true" aria-labelledby="${titleId}">
      <header class="repos-modal-head merge-head">
        <div class="merge-head-text">
          <h2 class="repos-modal-title" id="${titleId}">Levar commits para outro branch</h2>
          <p class="small muted mono merge-head-repo">${esc(repoName(opts.repo()))}</p>
        </div>
        <button type="button" class="btn ghost icon-only" data-action="close" aria-label="Fechar">${icon("x")}</button>
      </header>
      <ol class="merge-steps" aria-label="Etapas"></ol>
      <p class="sr-only" role="status" aria-live="polite" data-ref="live"></p>
      <div class="merge-body"></div>
      <footer class="merge-foot"></footer>
    </div>`;
  document.body.appendChild(backdrop);
  const dialog = backdrop.querySelector<HTMLElement>(".merge-modal")!;
  const stepsEl = dialog.querySelector<HTMLElement>(".merge-steps")!;
  const live = dialog.querySelector<HTMLElement>('[data-ref="live"]')!;
  const bodyEl = dialog.querySelector<HTMLElement>(".merge-body")!;
  const footEl = dialog.querySelector<HTMLElement>(".merge-foot")!;

  function render(): void {
    if (closed) return;
    const activeEl = document.activeElement as HTMLElement | null;
    const focusKey = activeEl && dialog.contains(activeEl) ? activeEl.dataset.focus : undefined;
    const scroll = bodyEl.scrollTop;
    stepsEl.innerHTML = STEPS.map(
      (s) =>
        `<li class="merge-step${s.n === step ? " current" : s.n < step ? " done" : ""}"${s.n === step ? ' aria-current="step"' : ""}>
          <span class="merge-step-n" aria-hidden="true">${s.n < step ? icon("check", 12) : s.n}</span><span>${esc(s.label)}</span>${
            s.n < step ? '<span class="sr-only"> (feito)</span>' : ""
          }</li>`
    ).join("");
    bodyEl.innerHTML = step === 1 ? stepWhat() : step === 2 ? stepPreview() : step === 3 ? stepConfirm() : stepDone();
    footEl.innerHTML = footHtml();
    if (!pendingFocus) bodyEl.scrollTop = scroll;
    const key = pendingFocus ?? focusKey;
    pendingFocus = null;
    if (key) dialog.querySelector<HTMLElement>(`[data-focus="${CSS.escape(key)}"]`)?.focus({ preventScroll: !!focusKey });
  }

  function goStep(next: Step): void {
    step = next;
    pendingFocus = "step-title";
    bodyEl.scrollTop = 0;
    render();
  }

  // ---------- Etapa 1 ----------

  function branchOptions(names: string[], chosen: string | null): string {
    return names
      .map((n) => {
        const b = infoOf(n);
        const tags = [b?.current ? "atual" : "", b?.is_default ? "padrão" : ""].filter(Boolean).join(", ");
        return `<option value="${esc(n)}"${n === chosen ? " selected" : ""}>${esc(tags ? `${n} (${tags})` : n)}</option>`;
      })
      .join("");
  }

  function stepWhat(): string {
    const names = localBranches().map((b) => b.name);
    const sources = names.includes(source) ? names : [source, ...names];
    const targets = sources.filter((n) => n !== source);
    if (target && !targets.includes(target)) targets.unshift(target);
    const disabled = applying ? " disabled" : "";
    const route = `<div class="merge-route">
      <label class="merge-field"><span class="field-label">De</span>
        <select class="select mono" data-action="source" data-focus="source"${disabled}>${branchOptions(sources, source)}</select></label>
      <span class="merge-arrow" aria-hidden="true">${icon("chevronRight", 16)}</span>
      <label class="merge-field"><span class="field-label">Para</span>
        ${
          targets.length
            ? `<select class="select mono" data-action="target" data-focus="target"${disabled}>${branchOptions(targets, target)}</select>`
            : `<span class="muted small merge-no-target">Não há outro branch neste Mac.</span>`
        }</label>
    </div>`;
    const t = target ? code(target) : "o destino";
    const modes = `<fieldset class="merge-modes">
      <legend class="field-label">Como levar</legend>
      ${modeOption("cherry_pick", "Só os commits que eu escolher", `Copia os commits marcados para ${t}. O branch ${code(source)} continua como está.`)}
      ${modeOption("merge", "Juntar o branch inteiro", `Leva para ${t} tudo o que ${code(source)} tem e ${t} ainda não tem.`)}
    </fieldset>`;
    return `<h3 class="merge-step-title" tabindex="-1" data-focus="step-title">O que você quer levar?</h3>
      ${route}
      ${target ? modes : ""}
      ${target ? commitsSection() : `<p class="callout warning">${icon("alert")}<span>Para levar commits, é preciso ter outro branch neste Mac.</span></p>`}`;
  }

  function modeOption(value: MergeMode, title: string, sub: string): string {
    return `<label class="merge-mode${mode === value ? " checked" : ""}">
      <input type="radio" name="merge-mode-${uid}" value="${value}" data-action="mode" data-focus="mode-${value}"${mode === value ? " checked" : ""}${
        applying ? " disabled" : ""
      } />
      <span class="merge-mode-text"><span class="merge-mode-title">${esc(title)}</span><span class="small muted">${sub}</span></span>
    </label>`;
  }

  function commitsSection(): string {
    if (previewError && !preview) {
      return `<div class="callout error push-callout-action" role="alert">${icon("alert")}<span>Não foi possível ler os commits: ${esc(previewError)}</span>
        <button type="button" class="btn" data-action="retry" data-focus="retry">${icon("refresh", 14)}<span>Tentar de novo</span></button></div>`;
    }
    if (!preview || preview.source !== source || preview.target !== target) {
      return `<div class="merge-commits-loading" aria-busy="true"><span class="sr-only">Lendo os commits…</span>${Array.from(
        { length: 3 },
        () => `<div class="skeleton push-skeleton-row"></div>`
      ).join("")}</div>`;
    }
    const list = preview.commits;
    const pick = mode === "cherry_pick";
    const n = selected?.size ?? 0;
    const pickable = list.filter((c) => !c.is_merge).length;
    const missing = missingCommit
      ? `<p class="callout merge-note">${icon("info", 14)}<span>O commit <span class="mono">${esc(missingCommit)}</span> já está na ${code(
          preview.target
        )} (ou não faz parte de ${code(preview.source)}).</span></p>`
      : "";
    if (list.length === 0) {
      return `${missing}<p class="callout success merge-uptodate">${icon("check")}<span>${code(preview.source)} não tem commits que a ${code(
        preview.target
      )} ainda não tenha.</span></p>`;
    }
    const head = `<div class="merge-commits-head">
      <h4 class="section-label merge-commits-label" id="merge-commits-${uid}">${icon("commit", 14)} Commits de ${esc(preview.source)} que ${esc(
        preview.target
      )} não tem <span class="push-commits-count">(${list.length.toLocaleString("pt-BR")}${list.length >= 200 ? "+" : ""})</span></h4>
      ${
        pick
          ? `<div class="merge-commits-tools">
              <span class="small muted" data-ref="count">${esc(`${n.toLocaleString("pt-BR")} de ${pickable.toLocaleString("pt-BR")} marcados`)}</span>
              <button type="button" class="btn ghost" data-action="all" data-focus="all"${n === pickable ? " disabled" : ""}>Marcar todos</button>
              <button type="button" class="btn ghost" data-action="none" data-focus="none"${n === 0 ? " disabled" : ""}>Desmarcar todos</button>
            </div>`
          : ""
      }
    </div>`;
    const rows = list.map((c) => commitRow(c, pick)).join("");
    return `${missing}<section class="merge-commits-section" aria-labelledby="merge-commits-${uid}">${head}
      <ul class="merge-commits">${rows}</ul>
      ${pick && n === 0 ? `<p class="small warn merge-pick-hint">${icon("alert", 12)} Marque pelo menos um commit.</p>` : ""}
    </section>`;
  }

  function commitRow(c: OutgoingCommit, pick: boolean): string {
    const meta = `<span class="merge-commit-meta small">
        <span class="mono push-sha">${esc(c.short_sha)}</span>
        <span title="${esc(`${c.author_name} <${c.author_email}>`)}">${esc(c.author_name || c.author_email)}</span>
        <time datetime="${esc(c.authored_at)}" title="${esc(fullDate(c.authored_at))}">${esc(relativeTime(c.authored_at))}</time>
        <span class="push-stat"><span class="push-ins">+${c.insertions.toLocaleString("pt-BR")}</span> <span class="push-del">−${c.deletions.toLocaleString(
          "pt-BR"
        )}</span><span class="sr-only"> linhas em ${esc(plural(c.files_changed, "arquivo", "arquivos"))}</span></span>
      </span>`;
    const subject = `<span class="merge-commit-subject">${c.is_merge ? `<span class="badge">Junção</span> ` : ""}${esc(c.subject || "(sem mensagem)")}</span>`;
    if (!pick) {
      return `<li class="merge-commit"><span class="merge-commit-dot" aria-hidden="true">${icon("commit", 14)}</span><span class="merge-commit-main">${subject}${meta}</span></li>`;
    }
    const on = selected?.has(c.sha) ?? false;
    const hint = c.is_merge ? `<span class="small muted merge-commit-why">Commits de junção não podem ser copiados sozinhos.</span>` : "";
    return `<li class="merge-commit${on ? " picked" : ""}${c.is_merge ? " disabled" : ""}">
      <label class="merge-commit-label">
        <input type="checkbox" data-action="pick" value="${esc(c.sha)}" data-focus="pick-${esc(c.sha)}"${on ? " checked" : ""}${
          c.is_merge || applying ? " disabled" : ""
        } />
        <span class="merge-commit-main">${subject}${meta}${hint}</span>
      </label>
    </li>`;
  }

  // ---------- Etapa 2 ----------

  function stepPreview(): string {
    const head = `<h3 class="merge-step-title" tabindex="-1" data-focus="step-title">Prévia: o que vai acontecer</h3>
      <p class="small muted merge-lead">${code(source)} <span aria-hidden="true">→</span><span class="sr-only">para</span> ${code(target ?? "")} · ${
        mode === "merge" ? "Juntar o branch inteiro" : "Só os commits escolhidos"
      }. Nada foi alterado ainda.</p>`;
    if (previewError) {
      return `${head}<div class="callout error push-callout-action" role="alert">${icon("alert")}<span>Não foi possível calcular a prévia: ${esc(previewError)}</span>
        <button type="button" class="btn" data-action="retry" data-focus="retry">${icon("refresh", 14)}<span>Tentar de novo</span></button></div>`;
    }
    if (!preview || previewKey !== requestKey()) {
      return `${head}<p class="merge-updating" aria-busy="true"><span class="spinner" aria-hidden="true"></span><span>Calculando a prévia…</span></p>`;
    }
    const p = preview;
    const updating = loading ? `<p class="merge-updating small"><span class="spinner" aria-hidden="true"></span><span>Atualizando a prévia…</span></p>` : "";
    const checks = p.checks.filter((c) => c.code !== "merge_conflict");
    const warnings = checks.some((c) => c.severity === "warning");
    return `${head}${updating}
      ${outcomeHtml(p)}
      ${
        p.strategy === "up_to_date" && !p.conflicts.length
          ? ""
          : `<ul class="merge-facts">
              <li>${icon("folder", 14)}<span>${whereText(p)}</span></li>
              <li>${icon("file", 14)}<span>${esc(plural(p.files_changed, "arquivo", "arquivos"))}, <span class="push-ins">+${p.insertions.toLocaleString(
                "pt-BR"
              )}</span> <span class="push-del">−${p.deletions.toLocaleString("pt-BR")}</span><span class="sr-only"> linhas</span>.</span></li>
            </ul>`
      }
      ${
        checks.length
          ? checkGroupsHtml(checks, {
              commits: p.commits,
              titles: { error: "Precisa resolver antes de continuar", warning: "Confira antes de continuar" },
              action: (k) => (CHECK_HINT[k] ? `<span class="small push-reason">${esc(CHECK_HINT[k]!)}</span>` : ""),
            })
          : ""
      }
      ${
        warnings && p.can_apply && !p.conflicts.length && p.strategy !== "up_to_date"
          ? `<label class="repos-check merge-ack"><input type="checkbox" data-action="ack" data-focus="ack"${ack ? " checked" : ""} /><span>Entendi os avisos e quero continuar</span></label>`
          : ""
      }
      ${techHtml(p)}`;
  }

  function outcomeHtml(p: MergePreview): string {
    if (p.conflicts.length) {
      const repo = opts.repo();
      return `<div class="callout error merge-outcome" role="alert">${icon("alert")}<div>
        <p class="merge-outcome-title"><strong>${esc(
          `Conflito em ${plural(p.conflicts.length, "arquivo", "arquivos")}`
        )}</strong></p>
        <p>Os dois branches mudaram as mesmas partes desses arquivos. Resolva no seu editor e volte aqui.</p>
        <ul class="merge-conflicts">${p.conflicts.map((f) => `<li><code>${esc(f)}</code></li>`).join("")}</ul>
        <div class="push-inline-actions">
          <button type="button" class="btn" data-action="finder" data-focus="finder"${repo.exists ? "" : " disabled"}>${icon("folder", 14)}<span>Abrir no Finder</span></button>
          <button type="button" class="btn" data-action="terminal" data-focus="terminal"${repo.exists ? "" : " disabled"}>${icon(
            "terminal",
            14
          )}<span>Abrir no Terminal</span></button>
          <button type="button" class="btn ghost" data-action="retry" data-focus="retry-conflict">${icon("refresh", 14)}<span>Ver a prévia de novo</span></button>
        </div>
      </div></div>`;
    }
    if (p.strategy === "up_to_date") {
      return `<div class="callout success merge-outcome">${icon("check")}<span><strong>${outcomeText(p)}</strong> Não há nada para levar.</span></div>`;
    }
    return `<div class="callout merge-outcome merge-outcome-ok">${icon(p.strategy === "fast_forward" ? "arrowUp" : "merge")}<span>${outcomeText(p)}</span></div>`;
  }

  function techHtml(p: MergePreview): string {
    const strategy = { fast_forward: "fast-forward", new_commits: p.mode === "merge" ? "merge commit" : "novos commits", up_to_date: "up to date" }[p.strategy];
    return `<details class="merge-tech"${techOpen ? " open" : ""}>
      <summary data-focus="tech">Detalhes técnicos</summary>
      <dl class="push-commit-people small">
        <dt>Modo</dt><dd class="mono">${p.mode === "merge" ? "merge" : "cherry-pick -x"}</dd>
        <dt>Resultado</dt><dd class="mono">${esc(strategy)}</dd>
        <dt>Origem</dt><dd class="mono">${esc(`${p.source} @ ${p.source_sha.slice(0, 12)}`)}</dd>
        <dt>Destino</dt><dd class="mono">${esc(`${p.target} @ ${p.target_sha.slice(0, 12)}`)}</dd>
        ${p.mode === "cherry_pick" ? `<dt>Ordem</dt><dd class="mono">${esc(p.selected.map((s) => s.slice(0, 7)).join(" → ") || "—")}</dd>` : ""}
      </dl>
    </details>`;
  }

  function previewBlock(): string | null {
    if (!target) return "Escolha o branch de destino.";
    if (nothingPicked()) return "Marque pelo menos um commit.";
    if (previewError) return "Não foi possível calcular a prévia.";
    if (!fresh()) return "Calculando a prévia…";
    const p = preview!;
    if (p.conflicts.length) return "Resolva os conflitos para continuar.";
    if (p.strategy === "up_to_date") return "Não há nada para levar.";
    if (!p.can_apply) return "Resolva os itens em “Precisa resolver antes de continuar”.";
    return null;
  }

  // ---------- Etapa 3 ----------

  function stepConfirm(): string {
    const p = preview!;
    const picked = mode === "cherry_pick" ? p.commits.filter((c) => selected?.has(c.sha)) : p.commits;
    const shown = picked.slice(0, CONFIRM_LIST_MAX);
    const warnings = [...new Set(p.checks.filter((c) => c.severity === "warning").map((c) => CHECK_TITLE[c.code] ?? c.code))];
    return `<h3 class="merge-step-title" tabindex="-1" data-focus="step-title">Confira e confirme</h3>
      <dl class="push-confirm">
        <dt>De</dt><dd>${code(source)}</dd>
        <dt>Para</dt><dd>${code(target ?? "")}</dd>
        <dt>O que acontece</dt><dd><span>${outcomeText(p)}</span></dd>
        <dt>Onde</dt><dd><span>${whereText(p)}</span></dd>
        <dt>Commits</dt><dd><ul class="merge-confirm-list">${shown
          .map((c) => `<li><span class="mono push-sha">${esc(c.short_sha)}</span> ${esc(c.subject || "(sem mensagem)")}</li>`)
          .join("")}${picked.length > shown.length ? `<li class="muted">${esc(`e mais ${picked.length - shown.length}`)}</li>` : ""}</ul></dd>
        <dt>Avisos</dt><dd>${
          warnings.length
            ? `<ul class="push-confirm-warnings">${warnings.map((w) => `<li>${icon("alert", 12)}${esc(w)}</li>`).join("")}</ul>`
            : `<span class="ok">${icon("check", 12)}Nenhum</span>`
        }</dd>
      </dl>
      ${
        applyError
          ? `<div class="callout error push-failed" role="alert">${icon("alert")}<div>
              <p class="push-failed-title"><strong>Não foi possível levar os commits.</strong> Nada foi alterado.</p>
              <p class="push-failed-msg">${esc(applyError)}</p>
              <div class="push-inline-actions">
                <button type="button" class="btn" data-action="repreview" data-focus="repreview">${icon("search", 14)}<span>Ver prévia de novo</span></button>
              </div>
            </div></div>`
          : ""
      }`;
  }

  // ---------- Etapa 4 ----------

  function stepDone(): string {
    const r = result!;
    const t = code(r.target);
    const what =
      r.strategy === "fast_forward"
        ? `A ${t} avançou até o último commit de ${code(source)}.`
        : r.strategy === "up_to_date"
          ? "Nada precisou mudar."
          : mode === "merge"
            ? `Foi criado 1 commit de junção na ${t}.`
            : `${r.created === 1 ? "Foi copiado 1 commit" : `Foram copiados ${r.created.toLocaleString("pt-BR")} commits`} para a ${t}.`;
    const repo = opts.repo();
    const dirty = !!repo.sync?.dirty;
    return `<div class="push-success merge-done">
      <div class="empty-icon push-success-icon" aria-hidden="true">${icon("check", 26)}</div>
      <h3 class="push-success-title merge-step-title" tabindex="-1" data-focus="step-title">Pronto! A ${t} agora tem esses commits.</h3>
      <p class="push-empty-sub">${what}</p>
      ${
        r.ahead_after > 0
          ? `<p class="push-empty-sub">${esc(
              r.ahead_after === 1 ? "1 commit da " : `${r.ahead_after.toLocaleString("pt-BR")} commits da `
            )}${t} ainda ${r.ahead_after === 1 ? "não está" : "não estão"} no GitHub.</p>`
          : `<p class="push-empty-sub">A ${t} já está igual no GitHub.</p>`
      }
      ${
        r.target !== repo.branch && dirty
          ? `<p class="small muted" id="merge-dirty-${uid}">Para mudar a pasta para ${t}, faça commit ou guarde (git stash) as alterações antes.</p>`
          : ""
      }
    </div>`;
  }

  // ---------- Rodapé ----------

  function footHtml(): string {
    const back = (to: Step, key: string): string =>
      `<button type="button" class="btn ghost" data-action="back" data-to="${to}" data-focus="${key}"${applying ? " disabled" : ""}>${icon(
        "chevronLeft",
        14
      )}<span>Voltar</span></button>`;
    const reason = (text: string | null): string =>
      `<p class="merge-foot-reason small${text ? "" : " sr-only"}" id="merge-reason-${uid}">${esc(text ?? "Tudo pronto.")}</p>`;
    if (step === 1) {
      const block = !target ? "Escolha o branch de destino." : nothingPicked() ? "Marque pelo menos um commit." : null;
      return `${reason(block)}
        <button type="button" class="btn ghost" data-action="close" data-focus="cancel">Cancelar</button>
        <button type="button" class="btn primary" data-action="next" data-to="2" data-focus="next1"${block ? " disabled" : ""} aria-describedby="merge-reason-${uid}">
          <span>Ver prévia</span>${icon("chevronRight", 14)}</button>`;
    }
    if (step === 2) {
      if (fresh() && preview!.strategy === "up_to_date" && !preview!.conflicts.length) {
        return `${reason(null)}${back(1, "back2")}<button type="button" class="btn primary" data-action="close" data-focus="close-ok">${icon(
          "check",
          14
        )}<span>Fechar</span></button>`;
      }
      const hasWarn = !!preview?.checks.some((c) => c.severity === "warning");
      const block = previewBlock() ?? (hasWarn && !ack ? "Confira os avisos e marque “Entendi” para continuar." : null);
      return `${reason(block)}${back(1, "back2")}
        <button type="button" class="btn primary" data-action="next" data-to="3" data-focus="next2"${block ? " disabled" : ""} aria-describedby="merge-reason-${uid}">
          <span>Continuar</span>${icon("chevronRight", 14)}</button>`;
    }
    if (step === 3) {
      const block = applyError ? "Veja a prévia de novo antes de tentar outra vez." : previewBlock();
      return `${reason(applying ? null : block)}${back(2, "back3")}
        <button type="button" class="btn primary" data-action="apply" data-focus="apply"${block || applying ? " disabled" : ""}${
          applying ? ' aria-busy="true"' : ""
        } aria-describedby="merge-reason-${uid}">${
          applying ? `<span class="spinner" aria-hidden="true"></span><span>Levando…</span>` : `${icon("merge", 14)}<span>${esc(applyLabel())}</span>`
        }</button>`;
    }
    const r = result!;
    const repo = opts.repo();
    const dirty = !!repo.sync?.dirty;
    const canSwitch = r.target !== repo.branch;
    return `${reason(null)}
      <button type="button" class="btn ghost" data-action="close" data-focus="close-done">Fechar</button>
      ${
        canSwitch
          ? `<button type="button" class="btn" data-action="checkout" data-focus="checkout"${dirty || checkingOut ? " disabled" : ""}${
              dirty ? ` aria-describedby="merge-dirty-${uid}"` : ""
            }>${checkingOut ? `<span class="spinner" aria-hidden="true"></span><span>Mudando…</span>` : `${icon("branch", 14)}<span>${esc(`Mudar para ${r.target}`)}</span>`}</button>`
          : ""
      }
      ${
        r.ahead_after > 0
          ? `<button type="button" class="btn primary" data-action="push" data-focus="push">${icon("send", 14)}<span>${esc(
              `Enviar ${r.target} para o GitHub`
            )}</span></button>`
          : ""
      }`;
  }

  // ---------- Ações ----------

  async function apply(): Promise<void> {
    if (applying || !target || !fresh() || previewBlock()) return;
    const p = preview!;
    applying = true;
    applyError = null;
    announce("Levando os commits…");
    render();
    try {
      const r = await reposApi.mergeApply(repoId, source, target, mode, commitsArg(), p.source_sha, p.target_sha);
      if (closed) return;
      applying = false;
      result = r;
      announce(`Pronto! A ${r.target} agora tem esses commits.`);
      void refreshBranches(repoId, false);
      opts.onApplied(r);
      goStep(4);
      return;
    } catch (e) {
      if (closed) return;
      applying = false;
      applyError = errMessage(e) || "Algo deu errado.";
      announce(`Não foi possível levar os commits: ${applyError}`);
      pendingFocus = "repreview";
    }
    render();
  }

  async function checkout(): Promise<void> {
    if (!result || checkingOut) return;
    const name = result.target;
    checkingOut = true;
    render();
    try {
      const row = await reposApi.checkout(repoId, name);
      if (closed) return;
      showToast(`Agora a pasta está no branch ${name}.`, "ok");
      opts.onCheckedOut(row);
      void refreshBranches(repoId, false);
      close();
    } catch (e) {
      checkingOut = false;
      showToast(errMessage(e) || "Não foi possível mudar de branch.", "err");
      render();
    }
  }

  function resetSelection(): void {
    selected = null;
    preview = null;
    previewKey = "";
    previewError = null;
    missingCommit = null;
  }

  function close(): void {
    if (closed) return;
    closed = true;
    active = null;
    window.clearTimeout(timer);
    window.clearTimeout(dataTimer);
    document.removeEventListener("keydown", onKey, true);
    document.removeEventListener(DATA_CHANGED_EVENT, onData);
    offBranches();
    unlisten?.();
    backdrop.remove();
    if (opener?.isConnected) opener.focus();
  }

  dialog.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled) return;
    const repo = opts.repo();
    switch (btn.dataset.action) {
      case "close":
        if (!applying) close();
        break;
      case "next":
      case "back":
        goStep(Number(btn.dataset.to) as Step);
        if (step === 2 && !fresh() && !loading) schedulePreview(0);
        break;
      case "retry":
        previewError = null;
        schedulePreview(0);
        break;
      case "all":
        if (preview) selected = new Set(preview.commits.filter((c) => !c.is_merge).map((c) => c.sha));
        missingCommit = null;
        schedulePreview();
        break;
      case "none":
        selected = new Set();
        schedulePreview();
        break;
      case "apply":
        void apply();
        break;
      case "repreview":
        applyError = null;
        goStep(2);
        schedulePreview(0);
        break;
      case "finder":
        reposApi.openInFinder(repo.root_path).catch((err) => showToast(errMessage(err), "err"));
        break;
      case "terminal":
        reposApi.openInTerminal(repo.root_path).catch((err) => showToast(errMessage(err), "err"));
        break;
      case "checkout":
        void checkout();
        break;
      case "push": {
        const t = result?.target;
        close();
        if (t) opts.onPushTarget(t);
        break;
      }
    }
  });

  dialog.addEventListener("change", (e) => {
    const input = e.target as HTMLInputElement | HTMLSelectElement;
    switch (input.dataset.action) {
      case "source":
        source = input.value;
        if (target === source) target = null;
        resetSelection();
        pendingFocus = "source";
        schedulePreview(0);
        break;
      case "target":
        target = input.value;
        resetSelection();
        pendingFocus = "target";
        schedulePreview(0);
        break;
      case "mode":
        mode = input.value as MergeMode;
        modeTouched = true;
        pendingFocus = `mode-${mode}`;
        schedulePreview();
        break;
      case "pick": {
        if (!selected) selected = new Set();
        if ((input as HTMLInputElement).checked) selected.add(input.value);
        else selected.delete(input.value);
        missingCommit = null;
        pendingFocus = `pick-${input.value}`;
        schedulePreview();
        break;
      }
      case "ack":
        ack = (input as HTMLInputElement).checked;
        pendingFocus = "ack";
        render();
        break;
    }
  });

  // `toggle` não sobe na árvore; a captura pega o evento no caminho.
  dialog.addEventListener(
    "toggle",
    (e) => {
      const d = e.target as HTMLElement;
      if (d instanceof HTMLDetailsElement && d.classList.contains("merge-tech")) techOpen = d.open;
    },
    true
  );

  const focusables = (): HTMLElement[] =>
    Array.from(dialog.querySelectorAll<HTMLElement>("button, a[href], input, select, summary, [tabindex]:not([tabindex='-1'])")).filter(
      (el) => !(el as HTMLButtonElement).disabled && el.offsetParent !== null
    );

  const onKey = (e: KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      if (!applying) close();
    } else if (e.key === "Tab") {
      const els = focusables();
      const first = els[0];
      const last = els[els.length - 1];
      if (!dialog.contains(document.activeElement)) {
        e.preventDefault();
        first?.focus();
      } else if (e.shiftKey && document.activeElement === first) {
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
    if (e.target === backdrop && !applying && step !== 3) close();
  });

  /** Algo mudou fora daqui (commit novo, envio, troca de branch): a prévia pode estar velha. */
  let dataTimer: number | undefined;
  const stale = (): void => {
    if (closed || applying || step > 2) return;
    window.clearTimeout(dataTimer);
    dataTimer = window.setTimeout(() => {
      if (!closed && !applying && step <= 2 && target && !nothingPicked()) schedulePreview(0);
    }, DEBOUNCE_MS);
  };
  const onData = (): void => stale();
  document.addEventListener(DATA_CHANGED_EVENT, onData);
  let unlisten: (() => void) | null = null;
  reposApi
    .onOutgoingChanged((id) => {
      if (id === repoId) stale();
    })
    .then((un) => {
      if (closed) un();
      else unlisten = un;
    })
    .catch(() => {});

  const sigNow = (): string =>
    localBranches()
      .map((b) => `${b.name}:${b.is_default ? 1 : 0}:${b.head_sha}`)
      .join(",");
  let branchSig = sigNow();
  const offBranches = onBranchesChange((id) => {
    if (id !== repoId || closed) return;
    const sig = sigNow();
    if (sig === branchSig) return;
    const first = branchSig === "";
    branchSig = sig;
    if (!target && pickTarget()) schedulePreview(0);
    else if (!first) stale();
    if (step === 1) render();
  });

  active = { focus: () => dialog.querySelector<HTMLElement>('[data-focus="step-title"]')?.focus() };
  pendingFocus = target || pickTarget() ? "target" : "source";
  schedulePreview(0);
  if (!dialog.contains(document.activeElement)) dialog.querySelector<HTMLElement>('[data-focus="step-title"]')?.focus();
}

function fullDate(iso: string): string {
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : new Date(t).toLocaleString("pt-BR", { dateStyle: "long", timeStyle: "short" });
}
