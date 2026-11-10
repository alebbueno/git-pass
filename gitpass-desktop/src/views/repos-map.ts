import "./repos-map.css";
import { api, type AccountRow } from "../lib/api";
import {
  reposApi,
  type MapProgress,
  type MapResult,
  type MapStatus,
  type MappedProject,
  type RepoPermission,
} from "../lib/api-repos";
import { errMessage, esc, shortenPath, showToast, withBusy } from "../lib/dom";
import { icon, type IconName } from "../lib/icons";
import { profileChip, profileName } from "../lib/profile";
import { DATA_CHANGED_EVENT, type MountView, type ViewContext } from "../lib/view";
import { suggestionMotivo } from "./repos-ui";

type RunStatus = "idle" | "running" | "done" | "error";
type Outcome = { ok: true } | { ok: false; error: string };

/** O último mapeamento fica guardado durante a sessão: dá para sair da tela e voltar. */
const state = {
  status: "idle" as RunStatus,
  root: "",
  progress: { phase: "scanning", found: 0 } as MapProgress,
  result: null as MapResult | null,
  error: "",
  /** Caminhos marcados para vincular. */
  selected: new Set<string>(),
  /** Conta escolhida pelo usuário (caminho → id da conta). */
  choice: new Map<string, string>(),
  /** Resultado do último «Vincular» de cada caminho. */
  outcomes: new Map<string, Outcome>(),
  lastBind: null as { ok: number; failed: number } | null,
  /** Grupos que o usuário abriu ou fechou. */
  open: new Map<MapStatus, boolean>(),
  query: "",
};

/** Avisa a tela montada (o mapeamento pode terminar com o usuário em outra tela). */
let onStateChange: ((what: "progress" | "all") => void) | null = null;

const PERM_RANK: Record<RepoPermission, number> = { pull: 0, triage: 1, push: 2, maintain: 3, admin: 4 };
const PERM_DETAIL: Record<RepoPermission, string> = {
  admin: "Administrador",
  maintain: "Manutenção",
  push: "Escrita",
  triage: "Triagem (só leitura)",
  pull: "Leitura",
};
const canPush = (p: RepoPermission): boolean => PERM_RANK[p] >= PERM_RANK.push;

type GroupDef = {
  status: MapStatus;
  title: string;
  icon: IconName;
  count: (n: number) => string;
  desc: string;
  open: boolean;
};

const GROUPS: GroupDef[] = [
  {
    status: "match",
    title: "Prontos para vincular",
    icon: "check",
    count: (n) => (n === 1 ? "pronto para vincular" : "prontos para vincular"),
    desc: "Encontramos estes repositórios nas suas contas. Sugerimos a conta com mais permissão em cada um.",
    open: true,
  },
  {
    status: "read_only",
    title: "Só leitura",
    icon: "lock",
    count: () => "só leitura",
    desc: "Suas contas podem baixar estes repositórios, mas não enviar alterações (push). Vincule só se quiser usar a conta para baixar atualizações.",
    open: true,
  },
  {
    status: "bound",
    title: "Já vinculados",
    icon: "repo",
    count: (n) => (n === 1 ? "já vinculado" : "já vinculados"),
    desc: "Estes projetos já usam uma conta do GitPass.",
    open: true,
  },
  {
    status: "not_found",
    title: "Sem acesso nas suas contas",
    icon: "alert",
    count: () => "sem acesso",
    desc: "Estes projetos apontam para o GitHub, mas nenhuma conta conectada tem acesso. Pode ser um repositório de outra pessoa, ou que foi apagado ou mudou de nome.",
    open: true,
  },
  {
    status: "other_remote",
    title: "Fora do GitHub",
    icon: "cloud",
    count: () => "fora do GitHub",
    desc: "O endereço remoto é de outro serviço (como GitLab ou Bitbucket). O GitPass só cuida de repositórios do GitHub.",
    open: false,
  },
  {
    status: "no_remote",
    title: "Sem remote",
    icon: "folder",
    count: () => "sem remote",
    desc: "Projetos Git sem endereço remoto. Publique o projeto no GitHub para poder vincular.",
    open: false,
  },
];

const BIND_CHUNK = 500;

function notify(what: "progress" | "all"): void {
  onStateChange?.(what);
}

function itemName(it: MappedProject): string {
  if (it.github_owner && it.github_repo) return `${it.github_owner}/${it.github_repo}`;
  const parts = it.root_path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? it.root_path;
}

function isSelectable(it: MappedProject): boolean {
  return it.status === "match" || it.status === "read_only";
}

function bestPermission(it: MappedProject): RepoPermission | null {
  let best: RepoPermission | null = null;
  for (const c of it.candidates) if (!best || PERM_RANK[c.permission] > PERM_RANK[best]) best = c.permission;
  return best;
}

function resetForRun(root: string): void {
  state.status = "running";
  state.root = root;
  state.progress = { phase: "scanning", found: 0 };
  state.result = null;
  state.error = "";
  state.selected.clear();
  state.choice.clear();
  state.outcomes.clear();
  state.lastBind = null;
  state.query = "";
}

async function runMap(root: string): Promise<void> {
  if (state.status === "running") return;
  resetForRun(root);
  notify("all");
  let unlisten: (() => void) | null = null;
  try {
    unlisten = await reposApi.onMapProgress((p) => {
      if (state.status !== "running") return;
      state.progress = p;
      notify("progress");
    });
  } catch {
    // Sem eventos, o progresso fica genérico; o mapeamento continua.
  }
  try {
    const r = await reposApi.mapFolder(root);
    state.result = r;
    state.root = r.root;
    for (const it of r.items) {
      if (it.status === "match" && it.suggested_account_id) state.selected.add(it.root_path);
    }
    state.status = "done";
  } catch (e) {
    state.status = "error";
    state.error = errMessage(e);
  } finally {
    unlisten?.();
    notify("all");
  }
}

/** Abre o seletor de pastas e começa o mapeamento na tela «Mapear pasta». */
export async function startFolderMap(ctx: ViewContext, btn?: HTMLButtonElement): Promise<void> {
  if (state.status === "running") {
    ctx.navigate("repos/map");
    return;
  }
  let path: string | null;
  try {
    path = btn ? await withBusy(btn, "Aguardando a pasta…", () => reposApi.pickFolder()) : await reposApi.pickFolder();
  } catch (e) {
    showToast(errMessage(e), "err");
    return;
  }
  if (!path) return;
  void runMap(path);
  ctx.navigate("repos/map");
}

function progressText(p: MapProgress): string {
  switch (p.phase) {
    case "scanning":
      if (p.found === 0) return "Procurando projetos…";
      return `Procurando projetos… ${p.found} ${p.found === 1 ? "encontrado" : "encontrados"}`;
    case "github":
      return `Consultando suas contas no GitHub… (${p.found} ${p.found === 1 ? "projeto encontrado" : "projetos encontrados"})`;
    default:
      return "Comparando os projetos com suas contas…";
  }
}

function formatSeconds(ms: number): string {
  return `${(ms / 1000).toLocaleString("pt-BR", { maximumFractionDigits: 1 })} s`;
}

export const mount: MountView = (root, ctx) => {
  let accounts: AccountRow[] = [];
  let disposed = false;
  let busy = false;

  const accountById = (id: string | null): AccountRow | undefined => accounts.find((a) => a.id === id);
  const who = (id: string): string => {
    const a = accountById(id);
    return a ? (a.login ? `@${a.login}` : profileName(a)) : "conta removida";
  };

  const items = (): MappedProject[] => state.result?.items ?? [];

  function choiceFor(it: MappedProject): string | null {
    const c = state.choice.get(it.root_path) ?? it.suggested_account_id;
    return c && it.candidates.some((x) => x.account_id === c) && accountById(c) ? c : null;
  }

  function visible(it: MappedProject): boolean {
    const q = state.query.trim().toLowerCase();
    if (!q) return true;
    const logins = [...it.candidates.map((c) => c.account_id), it.bound_account_id ?? ""]
      .map((id) => accountById(id))
      .flatMap((a) => (a ? [a.login ?? "", profileName(a)] : []));
    return [itemName(it), it.root_path, shortenPath(it.root_path), it.remote_url ?? "", ...logins].some((s) =>
      s.toLowerCase().includes(q)
    );
  }

  function bindable(): { path: string; account_id: string }[] {
    return items()
      .filter((it) => isSelectable(it) && state.selected.has(it.root_path))
      .map((it) => ({ path: it.root_path, account_id: choiceFor(it) }))
      .filter((x): x is { path: string; account_id: string } => !!x.account_id);
  }

  // ---------- Cabeçalho ----------

  function headerHtml(): string {
    let sub: string;
    if (state.status === "idle") sub = "Encontre todos os projetos de uma pasta e vincule cada um à conta certa.";
    else if (state.status === "done" && state.result) {
      const n = state.result.items.length;
      sub = `<span class="mono">${esc(shortenPath(state.root))}</span> · ${n === 1 ? "1 projeto" : `${n} projetos`} · ${esc(
        formatSeconds(state.result.elapsed_ms)
      )}`;
    } else sub = `Pasta: <span class="mono">${esc(shortenPath(state.root))}</span>`;

    const running = state.status === "running";
    const actions =
      state.status === "idle"
        ? `<button type="button" class="btn primary" data-action="pick">${icon("folder")}<span>Escolher pasta…</span></button>`
        : `<button type="button" class="btn" data-action="again"${running ? " disabled" : ""}>${icon("refresh")}<span>Mapear de novo</span></button>
           <button type="button" class="btn" data-action="pick"${running ? " disabled" : ""}>${icon("folder")}<span>Mapear outra pasta…</span></button>`;
    return `
      <header class="view-header">
        <div class="map-header-text">
          <button type="button" class="btn ghost map-back" data-action="back">${icon("repo", 14)}<span>Repositórios</span></button>
          <h1 class="view-title">Mapear pasta</h1>
          <p class="view-sub truncate" title="${esc(state.root)}">${sub}</p>
        </div>
        <div class="view-actions">${actions}</div>
      </header>`;
  }

  // ---------- Estados ----------

  function idleHtml(): string {
    const noAccounts = accounts.length === 0;
    return `
      <section class="panel">
        <div class="empty-state">
          <div class="empty-icon">${icon("folder", 26)}</div>
          <h3>Mapeie uma pasta inteira</h3>
          <p>Escolha a pasta onde ficam seus projetos (por exemplo, <span class="mono">~/Projects</span>). O GitPass procura os repositórios Git, confere nas suas contas do GitHub e sugere a conta certa para cada um.</p>
          ${
            noAccounts
              ? `<div class="callout warning map-callout" role="status">${icon("alert")}<span>Nenhuma conta conectada. Você pode mapear, mas só dá para vincular depois de conectar uma conta.</span></div>
                 <div class="map-empty-actions">
                   <button type="button" class="btn primary lg" data-action="go-accounts">${icon("user")}<span>Conectar conta</span></button>
                   <button type="button" class="btn lg" data-action="pick">${icon("folder")}<span>Escolher pasta…</span></button>
                 </div>`
              : `<div class="map-empty-actions"><button type="button" class="btn primary lg" data-action="pick">${icon("folder")}<span>Escolher pasta…</span></button></div>`
          }
        </div>
      </section>`;
  }

  function runningHtml(): string {
    return `
      <section class="panel map-progress" aria-busy="true" aria-labelledby="map-progress-text">
        <div class="map-progress-head">
          <span class="spinner" aria-hidden="true"></span>
          <p class="map-progress-text" id="map-progress-text" role="status" aria-live="polite">${esc(progressText(state.progress))}</p>
        </div>
        <ol class="map-steps">${stepsHtml()}</ol>
        <div class="map-progress-skeleton" aria-hidden="true">
          ${Array.from({ length: 4 }, () => `<div class="skeleton map-skeleton-row"></div>`).join("")}
        </div>
      </section>`;
  }

  function stepsHtml(): string {
    const order: MapProgress["phase"][] = ["scanning", "github", "matching"];
    const labels = ["Procurar projetos na pasta", "Consultar suas contas no GitHub", "Comparar e sugerir a conta"];
    const cur = order.indexOf(state.progress.phase);
    return labels
      .map((label, i) => {
        const st = i < cur ? "done" : i === cur ? "current" : "todo";
        const marker =
          st === "done"
            ? icon("check", 14)
            : st === "current"
              ? `<span class="spinner map-step-spinner" aria-hidden="true"></span>`
              : `<span aria-hidden="true">${i + 1}</span>`;
        const sr = st === "done" ? "Pronto: " : st === "current" ? "Agora: " : "Depois: ";
        return `<li class="map-step ${st}"><span class="map-step-marker">${marker}</span><span><span class="sr-only">${sr}</span>${esc(label)}</span></li>`;
      })
      .join("");
  }

  function updateProgress(): void {
    const text = root.querySelector<HTMLElement>("#map-progress-text");
    const steps = root.querySelector<HTMLElement>(".map-steps");
    if (!text || !steps) {
      render();
      return;
    }
    text.textContent = progressText(state.progress);
    steps.innerHTML = stepsHtml();
  }

  function errorHtml(): string {
    return `
      <div class="callout error" role="alert">${icon("alert")}<span>Não foi possível mapear a pasta: ${esc(state.error)}</span></div>
      <div class="map-empty-actions map-error-actions">
        <button type="button" class="btn primary" data-action="again">${icon("refresh")}<span>Tentar de novo</span></button>
        <button type="button" class="btn" data-action="pick">${icon("folder")}<span>Escolher outra pasta…</span></button>
      </div>`;
  }

  function emptyResultHtml(r: MapResult): string {
    return `
      <section class="panel">
        <div class="empty-state">
          <div class="empty-icon">${icon("search", 26)}</div>
          <h3>Nenhum projeto Git nesta pasta</h3>
          <p>Procuramos em até 6 níveis de subpastas, sem contar pastas ocultas e pastas como <span class="mono">node_modules</span>.${
            r.truncated ? " A busca parou antes do fim porque levou tempo demais." : ""
          } Escolha outra pasta.</p>
          <div class="map-empty-actions"><button type="button" class="btn primary lg" data-action="pick">${icon("folder")}<span>Escolher outra pasta…</span></button></div>
        </div>
      </section>`;
  }

  // ---------- Resultado ----------

  function doneHtml(r: MapResult): string {
    if (r.items.length === 0) return emptyResultHtml(r);
    return `
      <div class="map-status"></div>
      <div class="map-toolbar" role="group" aria-label="Filtro e seleção">
        <label class="map-search">
          <span class="sr-only">Filtrar projetos</span>
          ${icon("search")}
          <input type="search" class="input" placeholder="Filtrar por nome, pasta ou conta" value="${esc(state.query)}" autocomplete="off" spellcheck="false" />
        </label>
        <button type="button" class="btn" data-action="select-all" title="Marca todos os projetos prontos para vincular que aparecem na lista">${icon("check", 14)}<span>Marcar todos</span></button>
        <button type="button" class="btn" data-action="select-none">${icon("x", 14)}<span>Desmarcar todos</span></button>
      </div>
      <div class="map-groups"></div>
      <div class="map-footer"></div>`;
  }

  function statusHtml(r: MapResult): string {
    const parts: string[] = [];
    const counts = GROUPS.map((g) => ({ g, n: r.items.filter((it) => it.status === g.status).length })).filter((x) => x.n > 0);
    parts.push(`
      <section class="panel map-summary" aria-label="Resumo">
        <ul class="map-counts">
          ${counts
            .map(
              ({ g, n }) => `<li><button type="button" class="map-count map-count-${g.status}" data-action="jump" data-group="${g.status}">
                <span class="map-count-num">${n}</span><span class="map-count-label">${esc(g.count(n))}</span></button></li>`
            )
            .join(`<li class="map-count-sep" aria-hidden="true">·</li>`)}
        </ul>
      </section>`);

    if (state.lastBind) {
      const { ok, failed } = state.lastBind;
      if (ok > 0) {
        parts.push(
          `<div class="callout success map-callout" role="status">${icon("check")}<span>${
            ok === 1 ? "1 projeto vinculado." : `${ok} projetos vinculados.`
          } Os commits e pushes nessas pastas já usam a conta escolhida.</span></div>`
        );
      }
      if (failed > 0) {
        parts.push(
          `<div class="callout error map-callout" role="alert">${icon("alert")}<span>${
            failed === 1 ? "1 projeto não foi vinculado." : `${failed} projetos não foram vinculados.`
          } O motivo aparece em cada um, na lista abaixo.</span></div>`
        );
      }
    }

    if (r.truncated) {
      parts.push(
        `<div class="callout warning map-callout" role="status">${icon("alert")}<span>${
          r.limit_reached
            ? `Mostramos os primeiros ${r.max_projects} projetos. Para ver o resto, mapeie uma pasta menor.`
            : "A busca levou tempo demais e parou antes de olhar todas as pastas. Mapeie uma pasta menor. Se o macOS pediu acesso a uma pasta, permita e clique em «Mapear de novo»."
        }</span></div>`
      );
    }

    if (accounts.length === 0) {
      parts.push(`<div class="callout warning map-callout map-callout-action" role="status">${icon("alert")}<span>Nenhuma conta conectada. Conecte uma conta do GitHub para vincular os projetos.</span>
        <button type="button" class="btn" data-action="go-accounts">${icon("user", 14)}<span>Conectar conta</span></button></div>`);
    }

    const failedAccounts = r.accounts.filter((a) => !a.ok);
    if (failedAccounts.length > 0) {
      parts.push(`<div class="callout warning map-callout map-callout-action" role="status">${icon("alert")}<div>
        <p class="map-callout-title"><strong>${
          failedAccounts.length === 1 ? "Não foi possível consultar 1 conta no GitHub." : `Não foi possível consultar ${failedAccounts.length} contas no GitHub.`
        }</strong> Os projetos dessas contas aparecem em «Sem acesso nas suas contas».</p>
        <ul class="map-account-errors">${failedAccounts
          .map((a) => `<li><span class="mono">${esc(who(a.account_id))}</span>: ${esc(a.error ?? "erro desconhecido")}</li>`)
          .join("")}</ul>
      </div>
      <button type="button" class="btn" data-action="again">${icon("refresh", 14)}<span>Tentar de novo</span></button></div>`);
    }
    return parts.join("");
  }

  function permBadge(p: RepoPermission | null): string {
    if (!p) return "";
    return canPush(p)
      ? `<span class="badge success" title="${esc(PERM_DETAIL[p])}">${icon("check", 12)}Pode enviar</span>`
      : `<span class="badge" title="${esc(PERM_DETAIL[p])}">${icon("lock", 12)} Só leitura</span>`;
  }

  function rowMain(it: MappedProject, labelFor?: string): string {
    const tag = labelFor ? "label" : "span";
    const forAttr = labelFor ? ` for="${labelFor}"` : "";
    return `<${tag} class="map-row-main"${forAttr}>
      <span class="mono map-row-name truncate" title="${esc(itemName(it))}">${esc(itemName(it))}</span>
      <span class="mono muted small truncate" title="${esc(it.root_path)}">${esc(shortenPath(it.root_path))}</span>
    </${tag}>`;
  }

  function outcomeHtml(it: MappedProject): string {
    const o = state.outcomes.get(it.root_path);
    if (!o || o.ok) return "";
    return `<p class="map-row-error small" role="status">${icon("x", 14)}<span>Não foi vinculado: ${esc(o.error)}</span></p>`;
  }

  function accountPicker(it: MappedProject, i: number): string {
    const chosen = choiceFor(it);
    const options = it.candidates.filter((c) => accountById(c.account_id));
    if (options.length === 0) return `<span class="badge warning">${icon("alert", 12)}Conta removida</span>`;
    if (options.length === 1) {
      const a = accountById(options[0].account_id)!;
      return `<span class="map-row-account">${profileChip(a)}</span>`;
    }
    const chosenAcc = accountById(chosen);
    return `${chosenAcc ? `<span class="map-row-account">${profileChip(chosenAcc)}</span>` : ""}<select class="select map-account" data-index="${i}" data-focus="acc-${i}" aria-label="Conta para ${esc(itemName(it))}">
      ${options
        .map((c) => {
          const a = accountById(c.account_id)!;
          const label = `${profileName(a)}${a.login && a.label ? ` (@${a.login})` : ""} · ${canPush(c.permission) ? "Pode enviar" : "Só leitura"}`;
          return `<option value="${esc(a.id)}"${a.id === chosen ? " selected" : ""}>${esc(label)}</option>`;
        })
        .join("")}
    </select>`;
  }

  function selectableRow(it: MappedProject, i: number): string {
    const chosen = choiceFor(it);
    const checked = state.selected.has(it.root_path) && !!chosen;
    const perm = it.candidates.find((c) => c.account_id === chosen)?.permission ?? bestPermission(it);
    const motivo = suggestionMotivo(it.suggestion_reason, it.suggestion_label);
    const reason = motivo && chosen === it.suggested_account_id ? `<span class="map-reason muted small">${esc(motivo)}</span>` : "";
    return `<li class="list-row map-row${checked ? " selected" : ""}" data-index="${i}">
      <input type="checkbox" class="map-check" id="map-cb-${i}" data-index="${i}" data-focus="cb-${i}"${checked ? " checked" : ""}${
        chosen && !busy ? "" : " disabled"
      } />
      ${rowMain(it, `map-cb-${i}`)}
      <span class="map-row-meta">${reason}${permBadge(perm)}${accountPicker(it, i)}</span>
      ${outcomeHtml(it)}
    </li>`;
  }

  function boundRow(it: MappedProject, i: number): string {
    const bound = it.bound_account_id;
    const acc = accountById(bound);
    const chip = acc ? profileChip(acc) : `<span class="badge error">${icon("x", 12)}Conta removida</span>`;
    const justBound = state.outcomes.get(it.root_path)?.ok === true;
    const suggested = it.suggested_account_id;
    let status: string;
    let action = "";
    if (justBound) {
      status = `<span class="badge success">${icon("check", 12)}Vinculado agora</span>`;
    } else if (it.bound_matches_suggestion === true) {
      status = `<span class="badge success">${icon("check", 12)}Conta certa</span>`;
    } else if (it.bound_matches_suggestion === false && suggested && accountById(suggested)) {
      status = `<span class="badge warning">${icon("alert", 12)}${esc(who(suggested))} tem mais permissão</span>`;
      action = `<button type="button" class="btn map-row-action" data-action="switch" data-index="${i}" data-focus="sw-${i}"${
        busy ? " disabled" : ""
      }>${icon("user", 14)}<span>Trocar para ${esc(who(suggested))}</span></button>`;
    } else {
      status = `<span class="badge" title="Nenhuma conta conectada lista este repositório no GitHub">${icon("help", 12)}Não confirmado no GitHub</span>`;
    }
    return `<li class="list-row map-row" data-index="${i}">
      <span class="map-row-icon" aria-hidden="true">${icon("check", 14)}</span>
      ${rowMain(it)}
      <span class="map-row-meta">${status}${chip}${action}</span>
    </li>`;
  }

  function notFoundRow(it: MappedProject, i: number, failedLogins: Map<string, string>): string {
    const owner = (it.github_owner ?? "").toLowerCase();
    const failedOwner = failedLogins.get(owner);
    const hint = failedOwner
      ? `<span class="warn small">${icon("alert", 12)}Não foi possível consultar ${esc(failedOwner)}</span>`
      : `<span class="badge">${icon("lock", 12)} Sem acesso</span>`;
    return `<li class="list-row map-row" data-index="${i}">
      <span class="map-row-icon" aria-hidden="true">${icon("alert", 14)}</span>
      ${rowMain(it)}
      <span class="map-row-meta">${hint}</span>
    </li>`;
  }

  function infoRow(it: MappedProject, i: number): string {
    const remote = it.remote_url
      ? `<span class="mono muted small truncate map-row-remote" title="${esc(it.remote_url)}">${esc(it.remote_url)}</span>`
      : `<span class="muted small">Sem endereço remoto</span>`;
    return `<li class="list-row map-row" data-index="${i}">
      ${rowMain(it)}
      <span class="map-row-meta">${remote}</span>
    </li>`;
  }

  function groupHtml(g: GroupDef, r: MapResult): string {
    const all = r.items.map((it, i) => ({ it, i })).filter(({ it }) => it.status === g.status);
    if (all.length === 0) return "";
    const shown = all.filter(({ it }) => visible(it));
    if (shown.length === 0) return "";
    const failedLogins = new Map<string, string>();
    for (const a of r.accounts) {
      const acc = accountById(a.account_id);
      if (!a.ok && acc?.login) failedLogins.set(acc.login.toLowerCase(), who(a.account_id));
    }
    const rows = shown
      .map(({ it, i }) => {
        if (isSelectable(it)) return selectableRow(it, i);
        if (it.status === "bound") return boundRow(it, i);
        if (it.status === "not_found") return notFoundRow(it, i, failedLogins);
        return infoRow(it, i);
      })
      .join("");
    const open = state.open.get(g.status) ?? g.open;
    const count = shown.length === all.length ? `${all.length}` : `${shown.length} de ${all.length}`;
    const extra =
      g.status === "not_found"
        ? `<div class="map-group-actions"><button type="button" class="btn" data-action="go-accounts">${icon("plus", 14)}<span>Conectar outra conta</span></button></div>`
        : "";
    return `
      <section class="panel map-group map-group-${g.status}" data-group="${g.status}" aria-labelledby="map-g-${g.status}">
        <details${open ? " open" : ""}>
          <summary class="map-group-head" data-focus="g-${g.status}">
            <span class="map-group-icon" aria-hidden="true">${icon(g.icon, 16)}</span>
            <span class="map-group-title" id="map-g-${g.status}">${esc(g.title)}</span>
            <span class="badge map-group-count">${esc(count)}</span>
          </summary>
          <p class="muted small map-group-desc">${esc(g.desc)}</p>
          ${extra}
          <ul class="list map-list">${rows}</ul>
        </details>
      </section>`;
  }

  function renderStatus(): void {
    const box = root.querySelector<HTMLElement>(".map-status");
    if (box && state.result) box.innerHTML = statusHtml(state.result);
  }

  function renderGroups(): void {
    const box = root.querySelector<HTMLElement>(".map-groups");
    const r = state.result;
    if (!box || !r) return;
    const focusKey = (document.activeElement as HTMLElement | null)?.dataset?.focus;
    const html = GROUPS.map((g) => groupHtml(g, r)).join("");
    box.innerHTML =
      html ||
      `<p class="muted small map-no-results">${icon("search")} Nenhum projeto corresponde a “${esc(state.query.trim())}”.</p>`;
    box.querySelectorAll<HTMLDetailsElement>("details").forEach((d) => {
      d.addEventListener("toggle", () => {
        const g = d.closest<HTMLElement>("[data-group]")?.dataset.group as MapStatus | undefined;
        if (g) state.open.set(g, d.open);
      });
    });
    if (focusKey) box.querySelector<HTMLElement>(`[data-focus="${CSS.escape(focusKey)}"]`)?.focus();
  }

  function renderFooter(): void {
    const box = root.querySelector<HTMLElement>(".map-footer");
    if (!box) return;
    const n = bindable().length;
    const prevFocus = box.contains(document.activeElement);
    box.innerHTML = `
      <span class="map-footer-count small" aria-live="polite">${
        n === 0 ? "Nenhum projeto marcado" : n === 1 ? "1 projeto marcado" : `${n} projetos marcados`
      }</span>
      <button type="button" class="btn primary lg" data-action="bind-selected"${n === 0 || busy || accounts.length === 0 ? " disabled" : ""}>${icon(
        "plus"
      )}<span>Vincular selecionados (${n})</span></button>`;
    if (prevFocus) box.querySelector<HTMLButtonElement>("button")?.focus();
  }

  function renderResults(): void {
    renderStatus();
    renderGroups();
    renderFooter();
  }

  function render(): void {
    if (disposed) return;
    let body: string;
    switch (state.status) {
      case "idle":
        body = idleHtml();
        break;
      case "running":
        body = runningHtml();
        break;
      case "error":
        body = errorHtml();
        break;
      default:
        body = state.result ? doneHtml(state.result) : idleHtml();
    }
    root.innerHTML = `${headerHtml()}<div class="map-body">${body}</div>`;
    if (state.status === "done" && state.result && state.result.items.length > 0) {
      const input = root.querySelector<HTMLInputElement>(".map-search input")!;
      input.addEventListener("input", () => {
        state.query = input.value;
        renderGroups();
      });
      renderResults();
    }
  }

  // ---------- Ações ----------

  function setVisibleSelection(on: boolean): void {
    for (const it of items()) {
      if (!visible(it) || !isSelectable(it)) continue;
      if (on) {
        if (it.status === "match" && choiceFor(it)) state.selected.add(it.root_path);
      } else state.selected.delete(it.root_path);
    }
    renderGroups();
    renderFooter();
  }

  function jumpTo(status: MapStatus): void {
    if (state.query) {
      state.query = "";
      const input = root.querySelector<HTMLInputElement>(".map-search input");
      if (input) input.value = "";
    }
    state.open.set(status, true);
    renderGroups();
    const sec = root.querySelector<HTMLElement>(`.map-group[data-group="${status}"]`);
    sec?.scrollIntoView({ block: "start", behavior: "smooth" });
    sec?.querySelector<HTMLElement>("summary")?.focus({ preventScroll: true });
  }

  function markBound(it: MappedProject, bindingId: string, accountId: string): void {
    const best = bestPermission(it);
    const mine = it.candidates.find((c) => c.account_id === accountId)?.permission;
    it.status = "bound";
    it.binding_id = bindingId;
    it.bound_account_id = accountId;
    it.bound_matches_suggestion = best ? mine === best : null;
    if (it.bound_matches_suggestion) it.suggested_account_id = accountId;
  }

  async function bindSelected(btn: HTMLButtonElement): Promise<void> {
    const payload = bindable();
    if (payload.length === 0 || busy) return;
    busy = true;
    let ok = 0;
    let failed = 0;
    try {
      await withBusy(btn, `Vinculando ${payload.length === 1 ? "1 projeto" : `${payload.length} projetos`}…`, async () => {
        for (let i = 0; i < payload.length; i += BIND_CHUNK) {
          const chunk = payload.slice(i, i + BIND_CHUNK);
          const results = await reposApi.bindMany(chunk);
          for (const res of results) {
            const it = items().find((x) => x.root_path === res.path);
            if (!it) continue;
            if (res.repo) {
              const accountId = chunk.find((c) => c.path === res.path)?.account_id ?? res.repo.account_id;
              markBound(it, res.repo.id, accountId);
              state.outcomes.set(it.root_path, { ok: true });
              state.selected.delete(it.root_path);
              ok++;
            } else {
              state.outcomes.set(it.root_path, { ok: false, error: res.error ?? "Algo deu errado." });
              failed++;
            }
          }
        }
      });
    } catch (e) {
      showToast(`Não foi possível vincular: ${errMessage(e)}`, "err");
    } finally {
      busy = false;
    }
    if (disposed) return;
    state.lastBind = { ok, failed };
    if (ok > 0) {
      showToast(ok === 1 ? "1 projeto vinculado." : `${ok} projetos vinculados.`, "ok");
      ctx.notifyDataChanged();
    }
    if (failed > 0) showToast(failed === 1 ? "1 projeto não foi vinculado." : `${failed} projetos não foram vinculados.`, "err");
    renderResults();
    root.querySelector<HTMLElement>(".map-status")?.scrollIntoView({ block: "nearest" });
  }

  async function switchAccount(btn: HTMLButtonElement, it: MappedProject): Promise<void> {
    const target = it.suggested_account_id;
    if (!it.binding_id || !target || busy) return;
    busy = true;
    try {
      await withBusy(btn, "Trocando…", () => reposApi.changeAccount(it.binding_id!, target));
      it.bound_account_id = target;
      it.bound_matches_suggestion = true;
      showToast(`${itemName(it)} agora usa ${who(target)}. Identidade do Git atualizada.`, "ok");
      ctx.notifyDataChanged();
    } catch (e) {
      showToast(errMessage(e), "err");
    } finally {
      busy = false;
    }
    if (!disposed) renderGroups();
  }

  root.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled) return;
    const idx = Number(btn.dataset.index);
    switch (btn.dataset.action) {
      case "pick":
        void startFolderMap(ctx, btn);
        break;
      case "again":
        if (state.root) void runMap(state.root);
        break;
      case "back":
        ctx.navigate("repos");
        break;
      case "go-accounts":
        ctx.navigate("accounts");
        break;
      case "select-all":
        setVisibleSelection(true);
        break;
      case "select-none":
        setVisibleSelection(false);
        break;
      case "jump":
        if (btn.dataset.group) jumpTo(btn.dataset.group as MapStatus);
        break;
      case "bind-selected":
        void bindSelected(btn);
        break;
      case "switch": {
        const it = items()[idx];
        if (it) void switchAccount(btn, it);
        break;
      }
    }
  });

  root.addEventListener("change", (e) => {
    const el = e.target as HTMLElement;
    const it = items()[Number(el.dataset.index)];
    if (!it) return;
    if (el instanceof HTMLInputElement && el.classList.contains("map-check")) {
      if (el.checked) state.selected.add(it.root_path);
      else state.selected.delete(it.root_path);
      el.closest(".map-row")?.classList.toggle("selected", el.checked);
      renderFooter();
    } else if (el instanceof HTMLSelectElement && el.classList.contains("map-account")) {
      state.choice.set(it.root_path, el.value);
      renderGroups();
      renderFooter();
    }
  });

  async function loadAccounts(): Promise<void> {
    try {
      const accs = await api.accounts();
      if (disposed) return;
      accounts = accs;
    } catch {
      // Sem a lista de contas, os perfis não aparecem, mas o resultado continua visível.
    }
    if (!disposed && !busy) {
      if (state.status === "done") {
        // Só a lista muda: o filtro e o foco continuam onde estavam.
        if (root.querySelector(".map-groups")) renderResults();
        else render();
      } else if (state.status !== "running") render();
    }
  }

  onStateChange = (what) => {
    if (disposed) return;
    if (what === "progress" && state.status === "running") updateProgress();
    else render();
  };
  const onDataChanged = (): void => void loadAccounts();
  document.addEventListener(DATA_CHANGED_EVENT, onDataChanged);
  render();
  void loadAccounts();

  return () => {
    disposed = true;
    onStateChange = null;
    document.removeEventListener(DATA_CHANGED_EVENT, onDataChanged);
  };
};
