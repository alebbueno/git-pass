import type { AccountRow } from "../lib/api";
import type { CurrentRepo, CurrentRepoState } from "../lib/api-system";
import { systemApi } from "../lib/api-system";
import { errMessage, esc, relativeTime, shortenPath, showToast } from "../lib/dom";
import { codeChip, parseError } from "../lib/errors";
import { icon, stateIcon } from "../lib/icons";
import { closePopover, isPopoverOpenFor, openPopover, type PopoverHandle } from "../lib/popover";
import { profileChip, profileFamily, profileInitials, profileName } from "../lib/profile";
import { currentRepoName, onSystemState, pinRepo, refreshCurrentRepo, SOURCE_META, verdictMeta, type SystemSnapshot } from "../lib/system-state";
import type { ViewContext } from "../lib/view";

/** Marca de veredito: ícone + texto (a cor nunca é o único sinal). */
export function verdictPill(v: CurrentRepo["verdict"], size: "sm" | "md" = "sm"): string {
  const m = verdictMeta(v);
  return `<span class="gp-verdict gp-verdict-${m.kind} gp-verdict-${size}">${stateIcon(m.kind, size === "sm" ? 10 : 12)}<span>${esc(m.label)}</span></span>`;
}

export function sourceTag(c: CurrentRepo): string {
  const s = SOURCE_META[c.source];
  return `<span class="gp-source" title="${esc(s.detail)}">${icon(s.icon, 12)}<span>${esc(s.label)}</span></span>`;
}

/** Avatar do perfil da conta do repo (cor da família + iniciais); neutro sem conta. */
export function repoAvatar(c: Pick<CurrentRepo, "account_id">, accounts: AccountRow[], size: number): string {
  const acc = c.account_id ? accounts.find((a) => a.id === c.account_id) : undefined;
  if (!c.account_id) {
    return `<span class="avatar neutral" style="--size:${size}px" aria-hidden="true">${icon("repo")}</span>`;
  }
  const initials = acc ? profileInitials(profileName(acc)) : "?";
  return `<span class="avatar ${profileFamily(c.account_id)}" style="--size:${size}px" aria-hidden="true">${esc(initials)}</span>`;
}

/** Veredito em texto curto para o chip da barra superior. */
function chipVerdict(v: CurrentRepo["verdict"]): string {
  const m = verdictMeta(v);
  return `<span class="cr-verdict ${m.kind}">${stateIcon(m.kind, 12)}<span>${esc(m.label)}</span></span>`;
}

function accountLine(c: CurrentRepo, accounts: AccountRow[]): string {
  const acc = c.account_id ? accounts.find((a) => a.id === c.account_id) : undefined;
  if (acc) return profileChip(acc);
  if (c.account_id) return `<span class="badge error">${icon("x", 12)}Conta removida</span>`;
  return `<span class="badge">${icon("help", 12)}Sem conta vinculada</span>`;
}

/**
 * Chip compacto "Repo atual" na barra superior: cor do perfil, `owner/repo`,
 * de onde veio (fixado/foco/recente) e o veredito em texto. Abre o popover com os detalhes.
 */
export function mountCurrentRepoChip(host: HTMLElement, ctx: ViewContext, getAccounts: () => AccountRow[]): { refresh: () => void } {
  let snap: SystemSnapshot | null = null;
  let popover: PopoverHandle | null = null;

  const state = (): CurrentRepoState | null => (snap?.current.status === "ok" ? snap.current.value : null);

  function chipHtml(): string {
    const s = snap?.current;
    if (!s || s.status === "loading") {
      return `<span class="cr-chip is-empty" aria-busy="true"><span class="avatar neutral" style="--size:22px" aria-hidden="true"></span><span class="cr-label">Repo atual</span><span class="cr-name">Carregando…</span></span>`;
    }
    if (s.status === "error") {
      const p = parseError(s.error);
      return `<button type="button" class="cr-chip is-empty" data-cr="open" aria-haspopup="dialog" aria-expanded="false" title="${esc(
        p.unavailable ? "Recurso ainda não disponível" : "Clique para ver o motivo"
      )}" aria-label="Repo atual indisponível. Ver o motivo">
        <span class="avatar neutral" style="--size:22px" aria-hidden="true">${icon("repo")}</span>
        <span class="cr-label">Repo atual</span>
        <span class="cr-name">Indisponível</span>
        <span class="cr-verdict neutral">${icon("info", 12)}</span>
      </button>`;
    }
    const c = s.value.current;
    if (!c) {
      return `<button type="button" class="cr-chip is-empty" data-cr="open" aria-haspopup="dialog" aria-expanded="false" aria-label="Repo atual: nenhum. Abrir opções" title="Use o Git num repositório vinculado">
        <span class="avatar neutral" style="--size:22px" aria-hidden="true">${icon("repo")}</span>
        <span class="cr-label">Repo atual</span>
        <span class="cr-name truncate">Nenhum ainda</span>
      </button>`;
    }
    const name = currentRepoName(c);
    const src = SOURCE_META[c.source];
    const v = verdictMeta(c.verdict);
    return `<button type="button" class="cr-chip is-${v.kind}" data-cr="open" aria-haspopup="dialog" aria-expanded="false"
        aria-label="${esc(`Repo atual: ${name}. ${src.label}. ${v.label}. Abrir opções`)}" title="${esc(`${c.root_path}\n${src.label}`)}">
      ${repoAvatar(c, getAccounts(), 22)}
      <span class="cr-label">Repo atual</span>
      <span class="cr-name truncate">${esc(name)}</span>
      <span class="cr-origin" aria-hidden="true">${icon(src.icon, 13)}</span>
      ${chipVerdict(c.verdict)}
    </button>`;
  }

  function recentRow(r: CurrentRepo, st: CurrentRepoState, i: number): string {
    const name = currentRepoName(r);
    const pinned = r.binding_id !== null && st.pinned_binding_id === r.binding_id;
    return `<li class="gp-recent">
      <button type="button" class="gp-recent-main" data-cr="go" data-index="${i}"${r.binding_id ? "" : " disabled"} title="${esc(r.root_path)}">
        ${repoAvatar(r, getAccounts(), 22)}
        <span class="gp-recent-text">
          <span class="mono truncate">${esc(name)}</span>
          <span class="gp-recent-meta">${verdictPill(r.verdict)}<span class="muted">${esc(relativeTime(r.at))}</span></span>
        </span>
      </button>
      ${
        r.binding_id
          ? `<button type="button" class="btn ghost icon-only gp-recent-pin${pinned ? " is-pinned" : ""}" data-cr="${pinned ? "unpin" : "pin"}" data-index="${i}" aria-label="${esc(
              pinned ? `Desafixar ${name}` : `Fixar ${name} como atual`
            )}" title="${pinned ? "Desafixar" : "Fixar como atual"}">${icon(pinned ? "pinOff" : "pin", 14)}</button>`
          : ""
      }
    </li>`;
  }

  function popoverHtml(): string {
    const s = snap?.current;
    if (!s || s.status !== "ok") {
      const p = s?.status === "error" ? parseError(s.error) : null;
      return `<div class="gp-cr-pop">
        <p class="gp-cr-title">Repo atual indisponível</p>
        <p class="gp-cr-detail muted">${esc(p?.cause ?? "Carregando…")}</p>
        ${p?.code ? `<div class="gp-cr-verdict">${codeChip(p.code)}</div>` : ""}
        <div class="gp-cr-actions"><button type="button" class="btn" data-cr="retry">${icon("refresh", 14)}<span>Tentar de novo</span></button></div>
      </div>`;
    }
    const st = s.value;
    const c = st.current;
    const recents = st.recent.filter((r) => !c || r.root_path !== c.root_path);
    const head = c
      ? (() => {
          const src = SOURCE_META[c.source];
          const v = verdictMeta(c.verdict);
          const pinned = c.binding_id !== null && st.pinned_binding_id === c.binding_id;
          return `
          <div class="gp-cr-head">
            ${repoAvatar(c, getAccounts(), 36)}
            <div class="gp-cr-head-text">
              <p class="gp-cr-title mono">${esc(currentRepoName(c))}</p>
              <p class="gp-cr-path mono muted truncate" title="${esc(c.root_path)}">${esc(shortenPath(c.root_path))}</p>
            </div>
          </div>
          <div class="gp-cr-facts">
            ${accountLine(c, getAccounts())}
            ${sourceTag(c)}
          </div>
          <p class="gp-cr-detail muted">${esc(src.detail)}</p>
          <div class="gp-cr-verdict gp-cr-verdict-${v.kind}">
            ${verdictPill(c.verdict, "md")}
            <span class="gp-cr-verdict-text">${esc(c.message || v.detail)}</span>
            ${c.code ? codeChip(c.code) : ""}
          </div>
          <div class="gp-cr-actions">
            ${
              c.binding_id
                ? `<button type="button" class="btn primary" data-cr="go-current" data-autofocus>${icon("repo", 14)}<span>Abrir repositório</span></button>
                   <button type="button" class="btn" data-cr="${pinned ? "unpin-current" : "pin-current"}">${icon(pinned ? "pinOff" : "pin", 14)}<span>${pinned ? "Desafixar" : "Fixar"}</span></button>`
                : `<button type="button" class="btn primary" data-cr="go-repos" data-autofocus>${icon("plus", 14)}<span>Vincular em Repositórios</span></button>`
            }
          </div>`;
        })()
      : `<p class="gp-cr-title">Nenhum repo atual</p>
         <p class="gp-cr-detail muted">Quando você usar o Git num repositório vinculado, ele aparece aqui. Você também pode fixar um repositório para ele ficar sempre como atual.</p>
         <div class="gp-cr-actions"><button type="button" class="btn" data-cr="go-repos" data-autofocus>${icon("repo", 14)}<span>Ir para Repositórios</span></button></div>`;
    const list = recents.length
      ? `<p class="eyebrow gp-cr-section">Recentes</p><ul class="gp-recents">${recents.map((r) => recentRow(r, st, st.recent.indexOf(r))).join("")}</ul>`
      : "";
    const a11y = st.accessibility_granted
      ? ""
      : `<div class="gp-cr-note">
          ${icon("target", 14)}
          <p>Para seguir o app em foco (VS Code, Terminal), permita o GitPass em Acessibilidade. Sem isso, vale o fixado ou o mais recente.</p>
          <button type="button" class="btn ghost" data-cr="a11y">Abrir Ajustes</button>
        </div>`;
    return `<div class="gp-cr-pop">${head}${list}${a11y}</div>`;
  }

  function render(): void {
    const hadFocus = host.contains(document.activeElement);
    host.innerHTML = chipHtml();
    const btn = host.querySelector<HTMLButtonElement>('[data-cr="open"]');
    if (btn && popover) btn.setAttribute("aria-expanded", "true");
    if (hadFocus && !popover) btn?.focus();
  }

  function renderPopover(): void {
    if (!popover) return;
    const focusedAction = popover.el.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.cr : undefined;
    popover.el.querySelector(".gp-popover-body")!.innerHTML = popoverHtml();
    popover.reposition();
    if (focusedAction) popover.el.querySelector<HTMLElement>(`[data-cr="${focusedAction}"]`)?.focus();
  }

  async function setPin(id: string | null, btn: HTMLButtonElement): Promise<void> {
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

  function goRepo(id: string): void {
    closePopover();
    ctx.navigate(`repos/${encodeURIComponent(id)}`);
  }

  function onPopoverClick(e: Event): void {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-cr]");
    if (!btn || btn.disabled) return;
    const st = state();
    const i = Number(btn.dataset.index ?? -1);
    const row = st && i >= 0 ? st.recent[i] : null;
    const cur = st?.current ?? null;
    switch (btn.dataset.cr) {
      case "go-current":
        if (cur?.binding_id) goRepo(cur.binding_id);
        break;
      case "go":
        if (row?.binding_id) goRepo(row.binding_id);
        break;
      case "go-repos":
        closePopover();
        ctx.navigate("repos");
        break;
      case "pin-current":
        if (cur?.binding_id) void setPin(cur.binding_id, btn);
        break;
      case "unpin-current":
      case "unpin":
        void setPin(null, btn);
        break;
      case "pin":
        if (row?.binding_id) void setPin(row.binding_id, btn);
        break;
      case "a11y":
        systemApi.openAccessibilitySettings().catch((err) => showToast(errMessage(err)));
        break;
      case "retry":
        void refreshCurrentRepo();
        break;
    }
  }

  host.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>('[data-cr="open"]');
    if (!btn) return;
    if (isPopoverOpenFor(btn)) {
      closePopover();
      return;
    }
    popover = openPopover(btn, {
      label: "Repo atual",
      html: popoverHtml(),
      className: "gp-cr-popover",
      align: "end",
      onClose: () => {
        popover = null;
      },
    });
    popover.el.addEventListener("click", onPopoverClick);
  });

  onSystemState((s) => {
    snap = s;
    // Se o popover está aberto, o chip (âncora) fica; só o conteúdo muda.
    if (popover) {
      const btn = host.querySelector<HTMLButtonElement>('[data-cr="open"]');
      const next = document.createElement("div");
      next.innerHTML = chipHtml();
      const nb = next.querySelector<HTMLButtonElement>('[data-cr="open"]');
      if (btn && nb) {
        btn.className = nb.className;
        btn.innerHTML = nb.innerHTML;
        btn.setAttribute("aria-label", nb.getAttribute("aria-label") ?? "");
      }
      renderPopover();
    } else {
      render();
    }
  });

  return {
    refresh: () => {
      if (popover) renderPopover();
      else render();
    },
  };
}
