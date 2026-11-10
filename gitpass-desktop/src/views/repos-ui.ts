import type { AccountRow } from "../lib/api";
import type { RepoRow, SuggestionReason, WorkTreeStatusRow } from "../lib/api-repos";
import type { CurrentRepoState } from "../lib/api-system";
import { esc, shortenPath } from "../lib/dom";
import { icon } from "../lib/icons";
import { profileChip, profileFamily, profileName } from "../lib/profile";
import { SOURCE_META } from "../lib/system-state";

/** Peças partilhadas por Repositórios (lista), Painel, gaveta de detalhes e envio de commits. */

export type ProfileRef = Pick<AccountRow, "id" | "login" | "label">;

/** Por que um repositório entra em «Para revisar» (vazio = está tudo certo). */
export function reviewReasons(b: RepoRow, accounts: AccountRow[], st: CurrentRepoState | null): string[] {
  const out: string[] = [];
  const acc = accounts.find((a) => a.id === b.account_id);
  if (!b.exists) out.push("Pasta não encontrada");
  if (b.account_missing) out.push("Conta removida");
  else if (acc && !acc.has_token) out.push("Conta desconectada");
  if (b.exists && b.identity.state === "different") out.push("Identidade local diferente da conta");
  else if (b.exists && b.identity.state === "not_set") out.push("Identidade local não definida");
  if (b.remote_has_credentials) out.push("Senha no endereço remoto");
  const entry = st?.current?.binding_id === b.id ? st.current : st?.recent.find((r) => r.binding_id === b.id);
  if (entry?.verdict === "blocked") out.push("Bloqueado na última verificação");
  else if (entry?.verdict === "unconfirmed") out.push("Não confirmado na última verificação");
  return out;
}

export function aheadOf(b: RepoRow): number {
  return b.exists ? (b.sync?.ahead ?? 0) : 0;
}

/** Nome curto do projeto (o repositório no GitHub ou a pasta). */
function workHint(w: WorkTreeStatusRow): string {
  const parts: string[] = [];
  if (w.conflicted_files) parts.push(`${w.conflicted_files} conflito(s)`);
  if (w.staged_files) parts.push(`${w.staged_files} preparado(s)`);
  if (w.unstaged_files) parts.push(`${w.unstaged_files} alterado(s)`);
  if (w.untracked_files) parts.push(`${w.untracked_files} novo(s)`);
  if ((w.ahead ?? 0) > 0) parts.push(`${w.ahead} à frente`);
  if ((w.behind ?? 0) > 0) parts.push(`${w.behind} atrás`);
  return parts.join(" · ") || "Pendências";
}

export function projectName(b: Pick<RepoRow, "github_repo" | "root_path">): string {
  if (b.github_repo) return b.github_repo;
  const parts = b.root_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? b.root_path;
}

export function columnsHtml(): string {
  return `<div class="columns" aria-hidden="true"><span>Repositório local</span><span>Conta vinculada</span><span class="col-branch">Branch</span><span class="col-status">Identidade</span><span></span></div>`;
}

/** Linha da tabela de repositórios: abre a gaveta de detalhes. */
export function projectRowHtml(
  b: RepoRow,
  accounts: AccountRow[],
  st: CurrentRepoState | null,
  opts: { expanded?: boolean; tabbable?: boolean; highlightAhead?: boolean; work?: WorkTreeStatusRow | null } = {}
): string {
  const fam = b.account_missing ? "" : profileFamily(b.account_id);
  const reasons = reviewReasons(b, accounts, st);
  const ahead = aheadOf(b);
  const branch = b.branch ?? (b.detached_at ? "fora de um branch" : "—");
  const cur = st?.current?.binding_id === b.id ? st.current : null;
  const extra: string[] = [];
  if (ahead > 0) extra.push(plural(ahead, "commit para enviar", "commits para enviar"));
  if (b.exists && (b.sync?.behind ?? 0) > 0) extra.push(plural(b.sync!.behind, "commit novo no GitHub", "commits novos no GitHub"));
  if (b.exists && b.sync?.dirty) extra.push("alterações sem commit");
  const w = opts.work;
  if (w && !w.error) {
    if (w.conflicted_files) extra.push(plural(w.conflicted_files, "conflito", "conflitos"));
    if (w.staged_files) extra.push(plural(w.staged_files, "arquivo preparado", "arquivos preparados"));
    if (w.unstaged_files) extra.push(plural(w.unstaged_files, "arquivo alterado", "arquivos alterados"));
    if (w.untracked_files) extra.push(plural(w.untracked_files, "arquivo novo", "arquivos novos"));
    if ((w.ahead ?? 0) > 0 && ahead === 0) extra.push(plural(w.ahead!, "commit para enviar", "commits para enviar"));
    if ((w.behind ?? 0) > 0 && (b.sync?.behind ?? 0) === 0) extra.push(plural(w.behind!, "commit no remoto", "commits no remoto"));
  }
  const status = reasons.length
    ? `<span class="state warning" title="${esc(reasons.join(" · "))}">${icon("alert")}Revisar</span>`
    : `<span class="state">${icon("check")}Correta</span>`;
  const curTag = cur
    ? `<span class="row-current" title="${esc(`Repo atual · ${SOURCE_META[cur.source].label}`)}">${icon(SOURCE_META[cur.source].icon, 12)}Atual</span>`
    : "";
  const label = [projectName(b), b.account_missing ? "conta removida" : profileName(profileFor(accounts, b)), reasons.length ? `revisar: ${reasons.join(", ")}` : "identidade correta", ...extra]
    .filter(Boolean)
    .join(". ");
  const rowTone = opts.highlightAhead && ahead > 0 ? " has-pending-push" : "";
  return `<li><button type="button" class="project-row ${fam}${rowTone}" data-repo-id="${esc(b.id)}" aria-controls="repo-drawer" aria-expanded="${!!opts.expanded}" aria-label="${esc(
    label
  )}" tabindex="${opts.tabbable === false ? -1 : 0}" title="${esc(b.root_path)}">
    <span class="project-name"><span class="repo-mark${fam ? "" : " neutral"}">${icon("repo")}</span><span class="truncate">
      <strong class="truncate">${esc(projectName(b))}${ahead > 0 ? `<span class="row-ahead" aria-hidden="true">${icon("arrowUp", 11)}${ahead}</span>` : ""}</strong>
      <code class="truncate">${esc(shortenPath(b.root_path))}</code>
    </span></span>
    <span class="project-profile">${
      b.account_missing ? `<span class="state error">${icon("x")}Conta removida</span>` : profileChip(profileFor(accounts, b))
    }</span>
    <span class="project-branch">${icon("branch")}<span class="truncate">${esc(branch)}</span>${
      w && w.has_pending && !w.error
        ? `<span class="row-work hint" title="${esc(workHint(w))}">${icon("pullRequest", 12)}</span>`
        : ""
    }</span>
    <span class="project-status">${status}${curTag}</span>
    <span class="project-chevron" aria-hidden="true">${icon("chevronRight")}</span>
  </button></li>`;
}

export const REASON_TEXT: Record<SuggestionReason, string> = {
  owner_login: "conta dona do repositório",
  same_owner_binding: "já usada em outros repositórios deste dono",
  only_account: "única conta conectada",
  push_access: "única conta com permissão de push",
  folder_rule: "regra de pasta",
  org_rule: "regra de organização",
  folder_org_rule: "regra de pasta e organização",
};

export function suggestionMotivo(reason: SuggestionReason | null, label: string | null): string {
  if (label) return `Regra «${label}»`;
  if (reason) return REASON_TEXT[reason];
  return "";
}

export function repoName(b: Pick<RepoRow, "github_owner" | "github_repo" | "root_path">): string {
  if (b.github_owner && b.github_repo) return `${b.github_owner}/${b.github_repo}`;
  const parts = b.root_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? b.root_path;
}

export function identityText(name: string | null, email: string | null): string {
  if (!name && !email) return "não definida";
  return `${name ?? "(sem nome)"} <${email ?? "sem e-mail"}>`;
}

export function profileFor(accounts: AccountRow[], b: Pick<RepoRow, "account_id" | "account_login" | "account_label">): ProfileRef {
  return accounts.find((a) => a.id === b.account_id) ?? { id: b.account_id, login: b.account_login, label: b.account_label };
}

/** `@login` da conta (ou o nome do perfil); `fallback` quando a conta já não existe. */
export function whoText(accounts: AccountRow[], id: string, fallback = "outra conta"): string {
  const a = accounts.find((x) => x.id === id);
  return a ? (a.login ? `@${a.login}` : profileName(a)) : fallback;
}

export function plural(n: number, one: string, many: string): string {
  return `${n.toLocaleString("pt-BR")} ${n === 1 ? one : many}`;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toLocaleString("pt-BR", { maximumFractionDigits: v < 10 ? 1 : 0 })} ${units[i]}`;
}

// ---------- Modal ----------

type ModalOptions<T> = {
  title: string;
  body: string;
  confirmLabel: string;
  /** Ação destrutiva: o foco inicial fica em «Cancelar». */
  danger?: boolean;
  wide?: boolean;
  /** Lê o resultado do formulário ao confirmar; devolver `undefined` mantém o modal aberto. */
  read: (dialog: HTMLElement) => T | undefined;
};

let modalSeq = 0;

/** Modal de confirmação do kit; resolve com o valor de `read` ou `null` se for cancelado. */
export function openModal<T>(opts: ModalOptions<T>): Promise<T | null> {
  const opener = document.activeElement as HTMLElement | null;
  const titleId = `repos-modal-title-${++modalSeq}`;
  const backdrop = document.createElement("div");
  backdrop.className = "modal-backdrop repos-modal-backdrop";
  backdrop.innerHTML = `
    <div class="modal repos-modal${opts.wide ? " repos-modal-wide" : ""}" role="dialog" aria-modal="true" aria-labelledby="${titleId}">
      <header class="repos-modal-head">
        <h2 class="repos-modal-title" id="${titleId}">${esc(opts.title)}</h2>
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
      if (opener?.isConnected) opener.focus();
      resolve(value);
    };
    const focusables = (): HTMLElement[] =>
      Array.from(dialog.querySelectorAll<HTMLElement>("button, a[href], input, select, summary, [tabindex]:not([tabindex='-1'])")).filter(
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

// ---------- Menus ----------

export type MenuItem = {
  id: string;
  /** HTML já escapado. */
  html: string;
  checked?: boolean;
  disabled?: boolean;
  danger?: boolean;
};

type MenuOptions = {
  label: string;
  title?: string;
  items: MenuItem[];
  /** `menuitemradio` para escolher um perfil (o item marcado não é selecionável). */
  kind?: "action" | "radio";
  /** O seletor de perfis usa vidro; os outros menus ficam opacos. */
  glass?: boolean;
  onPick: (id: string) => void;
};

let activeMenu: { btn: HTMLButtonElement; close: () => void } | null = null;

export function closeMenus(): void {
  activeMenu?.close();
}

/** Menu suspenso junto de `btn` (o pai do botão recebe `.repos-menu-host`). Clicar de novo no botão fecha. */
export function openMenu(btn: HTMLButtonElement, opts: MenuOptions): void {
  if (activeMenu) {
    const same = activeMenu.btn === btn;
    activeMenu.close();
    if (same) return;
  }
  const radio = opts.kind === "radio";
  const host = btn.parentElement!;
  const menu = document.createElement("div");
  menu.className = `repos-menu${opts.glass ? " material" : " repos-menu-solid"}`;
  menu.setAttribute("role", "menu");
  menu.setAttribute("aria-label", opts.label);
  menu.innerHTML = `${opts.title ? `<p class="repos-menu-title muted small">${esc(opts.title)}</p>` : ""}${opts.items
    .map((it) => {
      const roleAttr = radio ? `role="menuitemradio" aria-checked="${!!it.checked}"` : `role="menuitem"`;
      const inactive = it.disabled || (radio && it.checked);
      return `<button type="button" class="repos-menu-item${it.danger ? " repos-danger-text" : ""}" ${roleAttr} data-item="${esc(it.id)}"${
        inactive ? ' aria-disabled="true"' : ""
      }>${it.html}</button>`;
    })
    .join("")}`;
  host.classList.add("repos-menu-host");
  host.appendChild(menu);
  btn.setAttribute("aria-expanded", "true");
  const items = Array.from(menu.querySelectorAll<HTMLButtonElement>(".repos-menu-item"));
  (items.find((i) => i.getAttribute("aria-disabled") !== "true") ?? items[0])?.focus();

  const onDocDown = (e: PointerEvent): void => {
    if (!menu.contains(e.target as Node) && !btn.contains(e.target as Node)) close();
  };
  const onKey = (e: KeyboardEvent): void => {
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
      btn.focus();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      items[(i + 1) % items.length]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      items[(i - 1 + items.length) % items.length]?.focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      items[0]?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      items[items.length - 1]?.focus();
    } else if (e.key === "Tab") {
      close();
    }
  };
  const close = (): void => {
    document.removeEventListener("pointerdown", onDocDown, true);
    menu.removeEventListener("keydown", onKey);
    menu.remove();
    if (btn.isConnected) btn.setAttribute("aria-expanded", "false");
    if (activeMenu?.close === close) activeMenu = null;
  };
  document.addEventListener("pointerdown", onDocDown, true);
  menu.addEventListener("keydown", onKey);
  menu.addEventListener("click", (e) => {
    const item = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-item]");
    if (!item?.dataset.item || item.getAttribute("aria-disabled") === "true") return;
    const id = item.dataset.item;
    close();
    btn.focus();
    opts.onPick(id);
  });
  activeMenu = { btn, close };
}

/** Seletor de perfis (vidro). `currentId` fica marcado e não é selecionável. */
export function openAccountMenu(
  btn: HTMLButtonElement,
  accounts: AccountRow[],
  title: string,
  currentId: string | null,
  onPick: (id: string) => void
): void {
  openMenu(btn, {
    label: title,
    title,
    kind: "radio",
    glass: true,
    items: accounts.map((a) => {
      const cur = a.id === currentId;
      return {
        id: a.id,
        checked: cur,
        html: `${profileChip(a)}${
          cur
            ? `<span class="repos-menu-current small">${icon("check", 14)} Atual</span>`
            : a.login && a.label
              ? `<span class="mono muted small truncate">@${esc(a.login)}</span>`
              : ""
        }`,
      };
    }),
    onPick,
  });
}
