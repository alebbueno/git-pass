import "./diagnostics.css";
import { integrationApi } from "../lib/api-integration";
import { systemApi, type DiagnosticItem, type DiagnosticState, type Diagnostics } from "../lib/api-system";
import { errMessage, esc, fullDateTime, relativeTime, showToast, withBusy } from "../lib/dom";
import { codeChip, errorView } from "../lib/errors";
import { icon, type IconName } from "../lib/icons";
import { announceDiagnostics, type Loadable } from "../lib/system-state";
import type { MountView } from "../lib/view";

type Group = { id: "error" | "warning" | "ok"; title: string; states: DiagnosticState[]; icon: IconName };

const GROUPS: Group[] = [
  { id: "error", title: "Erro", states: ["error"], icon: "xCircle" },
  { id: "warning", title: "Atenção", states: ["warning"], icon: "alert" },
  { id: "ok", title: "OK", states: ["ok", "info"], icon: "checkCircle" },
];

const STATE_META: Record<DiagnosticState, { label: string; icon: IconName; cls: string }> = {
  ok: { label: "OK", icon: "checkCircle", cls: "ok" },
  info: { label: "Informação", icon: "info", cls: "info" },
  warning: { label: "Atenção", icon: "alert", cls: "warning" },
  error: { label: "Erro", icon: "xCircle", cls: "error" },
};

const ITEM_ICON: Record<string, IconName> = {
  daemon: "activity",
  hooks: "shieldCheck",
  credentials: "key",
  git: "branch",
  keychain: "lock",
  ssh: "terminal",
  bindings: "repo",
  macos: "monitor",
  accessibility: "target",
};

const ACTIONS: Record<string, { label: string; icon: IconName }> = {
  repair_integration: { label: "Reparar integração", icon: "refresh" },
  restart_daemon: { label: "Reiniciar serviço", icon: "refresh" },
  open_accessibility: { label: "Abrir Acessibilidade", icon: "external" },
  open_accounts: { label: "Abrir Contas", icon: "user" },
  open_repos: { label: "Abrir Repositórios", icon: "repo" },
};

export const mount: MountView = (root, ctx) => {
  let alive = true;
  let data: Loadable<Diagnostics> = { status: "loading" };
  let exported: string | null = null;

  root.innerHTML = `
    <div class="page-head">
      <div class="heading-group"><h1>Diagnóstico</h1><p data-ref="sub">Verificando o GitPass neste Mac…</p></div>
      <div class="page-actions">
        <button type="button" class="control" data-action="export">${icon("download")}<span>Exportar logs…</span></button>
        <button type="button" class="control primary" data-action="rerun">${icon("refresh")}<span>Verificar de novo</span></button>
      </div>
    </div>
    <p class="sr-only" role="status" aria-live="polite" data-ref="live"></p>
    <div class="diag-page" data-ref="body"></div>`;

  const body = root.querySelector<HTMLElement>('[data-ref="body"]')!;
  const sub = root.querySelector<HTMLElement>('[data-ref="sub"]')!;
  const live = root.querySelector<HTMLElement>('[data-ref="live"]')!;

  function itemRow(it: DiagnosticItem): string {
    const m = STATE_META[it.state] ?? STATE_META.info;
    const action = it.action ? ACTIONS[it.action] : undefined;
    return `<li class="diag-item diag-item-${m.cls}">
      <span class="diag-item-state" aria-hidden="true">${icon(m.icon, 18)}</span>
      <span class="diag-item-kind" aria-hidden="true">${icon(ITEM_ICON[it.id] ?? "info", 14)}</span>
      <div class="diag-item-text">
        <p class="diag-item-label">${esc(it.label)}<span class="sr-only">: ${esc(m.label)}</span></p>
        <p class="diag-item-detail">${esc(it.detail)}</p>
      </div>
      <div class="diag-item-side">
        ${it.code ? codeChip(it.code) : ""}
        ${
          action
            ? `<button type="button" class="btn${it.state === "error" ? " primary" : ""}" data-diag-action="${esc(it.action!)}">${icon(action.icon, 14)}<span>${esc(
                action.label
              )}</span></button>`
            : ""
        }
      </div>
    </li>`;
  }

  function summary(d: Diagnostics): string {
    const count = (s: DiagnosticState[]) => d.items.filter((i) => s.includes(i.state)).length;
    const errors = count(["error"]);
    const warnings = count(["warning"]);
    const ok = count(["ok", "info"]);
    const tone = errors ? "error" : warnings ? "warning" : "success";
    const headline = errors
      ? `${errors} ${errors === 1 ? "problema impede" : "problemas impedem"} o GitPass de proteger seus commits`
      : warnings
        ? `${warnings} ${warnings === 1 ? "item pede" : "itens pedem"} atenção`
        : "Tudo certo com o GitPass neste Mac";
    return `
      <section class="surface diag-summary diag-summary-${tone}" aria-label="Resumo">
        <span class="diag-summary-icon" aria-hidden="true">${icon(errors ? "xCircle" : warnings ? "alert" : "shieldCheck", 26)}</span>
        <div class="diag-summary-text">
          <p class="diag-summary-title">${esc(headline)}</p>
          <p class="diag-summary-meta muted">Verificado ${esc(relativeTime(d.generated_at))} · <time datetime="${esc(d.generated_at)}">${esc(fullDateTime(d.generated_at))}</time></p>
        </div>
        <dl class="diag-counts">
          <div class="diag-count diag-count-error"><dt>Erro</dt><dd>${errors}</dd></div>
          <div class="diag-count diag-count-warning"><dt>Atenção</dt><dd>${warnings}</dd></div>
          <div class="diag-count diag-count-ok"><dt>OK</dt><dd>${ok}</dd></div>
        </dl>
      </section>`;
  }

  function versions(d: Diagnostics): string {
    const row = (label: string, value: string | null) =>
      `<div class="diag-version"><dt>${esc(label)}</dt><dd class="mono">${value ? esc(value) : `<span class="muted">não encontrado</span>`}</dd></div>`;
    return `
      <section class="surface diag-section" aria-labelledby="diag-h-versions">
        <h2 class="section-title" id="diag-h-versions">Versões</h2>
        <dl class="gp-group diag-versions">
          ${row("GitPass", d.app_version)}
          ${row("macOS", d.macos_version)}
          ${row("Git", d.git_version)}
        </dl>
      </section>`;
  }

  function exportedNote(): string {
    if (!exported) return "";
    return `<div class="callout success diag-exported" role="status">${icon("check", 16)}<div>
      <p class="diag-exported-title"><strong>Logs exportados.</strong> O arquivo não tem senhas, acessos nem chaves.</p>
      <p class="diag-exported-path"><span class="mono">${esc(exported)}</span>
        <button type="button" class="btn ghost icon-only" data-gp-copy="${esc(exported)}" aria-label="Copiar caminho do arquivo" title="Copiar caminho">${icon("copy", 14)}</button></p>
    </div></div>`;
  }

  function render(): void {
    if (!alive) return;
    if (data.status === "loading") {
      sub.textContent = "Verificando o GitPass neste Mac…";
      body.innerHTML = `${exportedNote()}<div class="skeleton diag-sk-summary"></div>
        <div class="gp-group diag-sk-list" aria-busy="true">${Array.from({ length: 5 }, () => `<div class="diag-sk-row"><div class="skeleton"></div></div>`).join("")}</div>`;
      return;
    }
    if (data.status === "error") {
      sub.textContent = "Não foi possível verificar.";
      body.innerHTML = `${exportedNote()}${errorView({
        title: "Não foi possível rodar o diagnóstico",
        error: data.error,
        actions: [{ label: "Tentar de novo", action: "rerun", icon: "refresh" }],
      })}`;
      return;
    }
    const d = data.value;
    const errors = d.items.filter((i) => i.state === "error").length;
    const warnings = d.items.filter((i) => i.state === "warning").length;
    sub.textContent = `${d.items.length} verificações · ${errors} com erro · ${warnings} pedem atenção`;
    const groups = GROUPS.map((g) => {
      const items = d.items.filter((i) => g.states.includes(i.state));
      if (!items.length) return "";
      return `<section class="surface diag-section diag-group diag-group-${g.id}" aria-labelledby="diag-h-${g.id}">
        <h2 class="section-title diag-group-title" id="diag-h-${g.id}">${icon(g.icon)}${esc(g.title)} · ${items.length}</h2>
        <ul class="gp-group diag-list">${items.map(itemRow).join("")}</ul>
      </section>`;
    }).join("");
    body.innerHTML = `${exportedNote()}${summary(d)}${groups}${versions(d)}`;
  }

  async function run(): Promise<void> {
    data = { status: "loading" };
    render();
    try {
      const d = await systemApi.diagnostics();
      if (!alive) return;
      data = { status: "ok", value: d };
      announceDiagnostics(d);
      const errors = d.items.filter((i) => i.state === "error").length;
      live.textContent = errors ? `Diagnóstico pronto: ${errors} com erro.` : "Diagnóstico pronto.";
    } catch (error) {
      if (!alive) return;
      data = { status: "error", error };
    }
    render();
  }

  async function exportLogs(btn: HTMLButtonElement): Promise<void> {
    try {
      const path = await withBusy(btn, "Exportando…", () => systemApi.exportLogs());
      if (!alive || !path) return;
      exported = path;
      render();
      live.textContent = "Logs exportados.";
    } catch (e) {
      showToast(errMessage(e));
    }
  }

  async function runAction(btn: HTMLButtonElement, action: string): Promise<void> {
    switch (action) {
      case "repair_integration":
        try {
          const r = await withBusy(btn, "Reparando…", () => integrationApi.apply(false));
          showToast(r.warnings.length ? r.warnings.join("\n") : "Integração reparada.", r.warnings.length ? "err" : "ok");
        } catch (e) {
          showToast(errMessage(e));
        }
        await run();
        break;
      case "restart_daemon":
        try {
          await withBusy(btn, "Reiniciando…", () => integrationApi.daemonRestart());
          showToast("Serviço em segundo plano reiniciado.", "ok");
        } catch (e) {
          showToast(errMessage(e));
        }
        await run();
        break;
      case "open_accessibility":
        systemApi.openAccessibilitySettings().catch((e) => showToast(errMessage(e)));
        break;
      case "open_accounts":
        ctx.navigate("accounts");
        break;
      case "open_repos":
        ctx.navigate("repos");
        break;
    }
  }

  root.addEventListener("click", (ev) => {
    const t = ev.target as HTMLElement;
    const act = t.closest<HTMLButtonElement>("button[data-diag-action]");
    if (act && !act.disabled) {
      void runAction(act, act.dataset.diagAction!);
      return;
    }
    const btn = t.closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled) return;
    if (btn.dataset.action === "rerun") {
      const header = root.querySelector<HTMLButtonElement>('.view-header [data-action="rerun"]')!;
      void withBusy(header, "Verificando…", run);
    } else if (btn.dataset.action === "export") {
      void exportLogs(btn);
    }
  });

  void run();

  return () => {
    alive = false;
  };
};
