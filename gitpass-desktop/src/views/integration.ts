import "./integration.css";
import {
  integrationApi,
  integrationHeadline,
  onIntegrationChanged,
  type CheckState,
  type ComponentStatus,
  type IntegrationReport,
  type IntegrationStatus,
} from "../lib/api-integration";
import { systemApi, type AuditEntry, type AuditKind, type EmergencyState } from "../lib/api-system";
import { clockTime, errMessage, esc, fullDateTime, relativeTime, showToast, withBusy } from "../lib/dom";
import { codeChip, errorView } from "../lib/errors";
import { icon, stateIcon } from "../lib/icons";
import {
  AUDIT_META,
  emergencyRemaining,
  formatCountdown,
  isEmergencyActive,
  onSystemState,
  refreshEmergency,
  startEmergency,
  stopEmergency,
  type Loadable,
} from "../lib/system-state";
import type { MountView } from "../lib/view";
import { emergencySentence } from "../shell/emergency-banner";
import { openModal } from "./repos-ui";

const CHECK_BADGES: Record<CheckState, { cls: "success" | "warning" | "error"; label: string }> = {
  ok: { cls: "success", label: "Ativo" },
  missing: { cls: "error", label: "Faltando" },
  outdated: { cls: "warning", label: "Desatualizado" },
  conflict: { cls: "warning", label: "Conflito" },
  error: { cls: "error", label: "Erro" },
};

type CheckId = "credentials" | "hooks" | "daemon";

const CHECKS: { id: CheckId; label: string; icon: "key" | "shieldCheck" | "activity"; current: string; previous: string }[] = [
  { id: "credentials", label: "Credenciais do GitHub", icon: "key", current: "Configurado agora", previous: "Substituído (salvo para restaurar)" },
  { id: "hooks", label: "Verificação nos commits", icon: "shieldCheck", current: "Pasta de verificação atual", previous: "Também roda as verificações em" },
  { id: "daemon", label: "Serviço em segundo plano", icon: "activity", current: "Atual", previous: "Anterior" },
];

const DURATIONS: { minutes: number; label: string }[] = [
  { minutes: 15, label: "15 min" },
  { minutes: 60, label: "1 h" },
  { minutes: 240, label: "4 h" },
];

type AuditFilter = "all" | "blocked" | "unconfirmed" | "emergency" | "ci";

const AUDIT_FILTERS: { id: AuditFilter; label: string; kinds: AuditKind[] | null }[] = [
  { id: "all", label: "Todos", kinds: null },
  { id: "blocked", label: "Bloqueios", kinds: ["blocked"] },
  { id: "unconfirmed", label: "Não confirmados", kinds: ["unconfirmed"] },
  { id: "emergency", label: "Emergência", kinds: ["emergency_on", "emergency_off", "emergency_expired", "commit_allowed_emergency"] },
  { id: "ci", label: "CI", kinds: ["ci_bypass"] },
];

function checkBadge(state: CheckState): string {
  const b = CHECK_BADGES[state];
  return `<span class="badge ${b.cls}">${stateIcon(b.cls)}${esc(b.label)}</span>`;
}

function checkValue(label: string, value: string | null): string {
  if (!value) return "";
  return `<p class="int-check-value"><span class="muted">${esc(label)}:</span> <span class="mono">${esc(value)}</span></p>`;
}

export function auditRow(e: AuditEntry): string {
  const m = AUDIT_META[e.kind] ?? { label: e.kind, kind: "info" as const, icon: "info" as const };
  return `<li class="int-audit-row">
    <span class="badge ${m.kind} int-audit-kind">${icon(m.icon, 12)}${esc(m.label)}</span>
    <div class="int-audit-main">
      <p class="int-audit-msg">${esc(e.message)}</p>
      ${
        e.repo || e.code
          ? `<p class="int-audit-meta">${e.repo ? `<span class="mono truncate">${esc(e.repo)}</span>` : ""}${e.code ? codeChip(e.code) : ""}</p>`
          : ""
      }
    </div>
    <time class="int-audit-time" datetime="${esc(e.at)}" title="${esc(fullDateTime(e.at))}">${esc(relativeTime(e.at))}</time>
  </li>`;
}

export const mount: MountView = (root) => {
  let alive = true;
  let intStatus: Loadable<IntegrationStatus> = { status: "loading" };
  let intBusy = false;
  let emergency: Loadable<EmergencyState> = { status: "loading" };
  let audit: Loadable<AuditEntry[]> = { status: "loading" };
  let auditFilter: AuditFilter = "all";
  let duration = 60;
  let reasonDraft = "";
  let tick: number | undefined;

  root.innerHTML = `
    <div class="page-head">
      <div class="heading-group"><h1>Integração</h1><p>Conexão com o Git, modo emergência e auditoria.</p></div>
      <button type="button" class="control" data-action="refresh-all">${icon("refresh")}<span>Verificar de novo</span></button>
    </div>
    <div class="int-page">
      <section class="surface int-section" aria-labelledby="int-h-git">
        <h2 class="section-title" id="int-h-git">Integração com o Git</h2>
        <div data-ref="integration" aria-live="polite"></div>
      </section>

      <section class="surface int-section" aria-labelledby="int-h-wrapper">
        <h2 class="section-title" id="int-h-wrapper">Atalho do Git (opcional)</h2>
        <div class="gp-group">
          <div class="gp-group-row">
            <span class="int-row-icon" aria-hidden="true">${icon("terminal", 16)}</span>
            <div class="gp-row-text">
              <p class="gp-row-label">Proteção para ferramentas que ignoram as verificações</p>
              <p class="gp-row-desc">Alguns apps rodam o Git sem passar pelas verificações. Um atalho opcional do <span class="mono">git</span> no seu PATH vai conferir a conta antes de cada commit e envio. Ele nunca substitui o Git do sistema e só é instalado se você pedir.</p>
            </div>
            <div class="gp-row-control"><span class="badge info">${icon("clock", 12)}Em breve</span></div>
          </div>
        </div>
      </section>

      <section class="surface int-section" aria-labelledby="int-h-emergency">
        <h2 class="section-title" id="int-h-emergency">Modo emergência</h2>
        <div data-ref="emergency"></div>
      </section>

      <section class="surface int-section" aria-labelledby="int-h-audit">
        <div class="section-head">
          <h2 class="section-title" id="int-h-audit">Auditoria</h2>
          <button type="button" class="control ghost" data-action="refresh-audit">${icon("refresh")}<span>Atualizar</span></button>
        </div>
        <p class="int-audit-lead caption muted">Registro local de bloqueios, avisos e do modo emergência. Não guarda senhas nem acessos.</p>
        <div class="segmented int-audit-filters" role="group" aria-label="Filtrar auditoria por tipo" data-ref="audit-filters"></div>
        <p class="sr-only" role="status" aria-live="polite" data-ref="audit-count"></p>
        <div data-ref="audit"></div>
      </section>
    </div>`;

  const $ = (ref: string) => root.querySelector<HTMLElement>(`[data-ref="${ref}"]`)!;
  const intEl = $("integration");
  const emEl = $("emergency");
  const auditEl = $("audit");

  // ---------- Integração com o Git ----------

  function checkRow(def: (typeof CHECKS)[number], c: ComponentStatus, s: IntegrationStatus): string {
    const dis = intBusy ? " disabled" : "";
    let action = "";
    if (def.id === "hooks" && c.state === "conflict") {
      action = `<button type="button" class="btn" data-int="chain-hooks"${dis}>Manter as duas</button>`;
    } else if (def.id === "daemon" && s.enabled && c.state !== "missing" && s.binaries.state !== "error") {
      action = `<button type="button" class="btn" data-int="daemon-restart"${dis}>${icon("refresh", 14)}<span>Reiniciar</span></button>`;
    }
    return `
      <div class="gp-group-row int-check" role="group" aria-label="${esc(def.label)}">
        <span class="int-row-icon int-row-icon-${CHECK_BADGES[c.state].cls}" aria-hidden="true">${icon(def.icon, 16)}</span>
        <div class="gp-row-text">
          <p class="gp-row-label">${esc(def.label)}</p>
          <p class="gp-row-desc">${esc(c.summary)}</p>
          ${checkValue(def.current, c.current_value)}
          ${checkValue(def.previous, c.previous_value)}
        </div>
        <div class="gp-row-control">${action}${checkBadge(c.state)}</div>
      </div>`;
  }

  function renderIntegration(): void {
    if (!alive) return;
    intEl.setAttribute("aria-busy", String(intBusy || intStatus.status === "loading"));
    if (intStatus.status === "loading") {
      intEl.innerHTML = `<div class="gp-group"><div class="gp-group-row"><span class="spinner" aria-hidden="true"></span><span class="muted">Verificando a integração…</span></div></div>`;
      return;
    }
    if (intStatus.status === "error") {
      intEl.innerHTML = errorView({
        title: "Não foi possível verificar a integração",
        error: intStatus.error,
        actions: [{ label: "Tentar de novo", action: "int-retry", icon: "refresh" }],
      });
      return;
    }
    const s = intStatus.value;
    const h = integrationHeadline(s);
    const dis = intBusy ? " disabled" : "";
    const callouts: string[] = [];
    if (!s.enabled) {
      callouts.push(
        `<div class="callout warning">${icon("alert", 16)}<span>A integração está desativada: o Git funciona como antes de você instalar o GitPass. Ative para voltar a usar a conta certa em cada repositório.</span></div>`
      );
    }
    if (!s.git_found) callouts.push(`<div class="callout error">${icon("x", 16)}<span>${esc(s.credentials.summary)}</span></div>`);
    else if (s.binaries.state === "error") callouts.push(`<div class="callout error">${icon("x", 16)}<span>${esc(s.binaries.summary)}</span></div>`);
    const intro = s.ready
      ? "O Git usa automaticamente a conta vinculada a cada repositório."
      : "O GitPass configura o Git sozinho ao abrir. Se algo mudar, clique em Reparar.";
    intEl.innerHTML = `
      <div class="int-summary int-summary-${h.kind}">
        <span class="int-summary-icon" aria-hidden="true">${icon(h.kind === "success" ? "shieldCheck" : "shield", 22)}</span>
        <div class="int-summary-text">
          <p class="int-summary-title">Integração com o Git · ${esc(h.label)}</p>
          <p class="int-summary-desc">${esc(intro)}</p>
        </div>
        <div class="int-summary-actions">
          ${s.enabled ? `<button type="button" class="btn ghost" data-int="undo"${dis}>Desfazer integração</button>` : ""}
          <button type="button" class="btn${s.ready && s.enabled ? "" : " primary"}" data-int="repair"${dis}>${icon("refresh", 14)}<span>${s.enabled ? "Reparar" : "Ativar integração"}</span></button>
        </div>
      </div>
      ${callouts.length ? `<div class="int-callouts">${callouts.join("")}</div>` : ""}
      <div class="gp-group">${CHECKS.map((d) => checkRow(d, s[d.id], s)).join("")}</div>`;
  }

  async function loadIntegration(): Promise<void> {
    try {
      intStatus = { status: "ok", value: await integrationApi.status() };
    } catch (error) {
      intStatus = { status: "error", error };
    }
    renderIntegration();
  }

  const reportResult = (r: IntegrationReport, okMessage: string): void => {
    if (r.warnings.length) showToast(r.warnings.join("\n"));
    else showToast(r.changes.length ? `${okMessage}\n${r.changes.join("\n")}` : "Tudo certo. Nada para mudar.", "ok");
  };

  async function runIntegration(btn: HTMLButtonElement, busyLabel: string, task: () => Promise<void>): Promise<void> {
    if (intBusy) return;
    intBusy = true;
    for (const b of intEl.querySelectorAll<HTMLButtonElement>("[data-int]")) if (b !== btn) b.disabled = true;
    try {
      await withBusy(btn, busyLabel, task);
    } catch (e) {
      showToast(errMessage(e));
    } finally {
      intBusy = false;
      await loadIntegration();
    }
  }

  async function onIntegrationAction(btn: HTMLButtonElement): Promise<void> {
    const action = btn.dataset.int;
    const s = intStatus.status === "ok" ? intStatus.value : null;
    if (action === "repair") {
      await runIntegration(btn, "Reparando…", async () => reportResult(await integrationApi.apply(false), "Integração reparada."));
    } else if (action === "chain-hooks") {
      const path = s?.hooks.current_value ?? "";
      const ok = await openModal<boolean>({
        title: "Manter também as suas verificações?",
        body: `<p class="repos-modal-lead">O Git vai usar as verificações do GitPass, que conferem a conta e depois rodam as suas verificações em <span class="mono">${esc(
          path
        )}</span>.</p><p class="repos-modal-lead muted">Para voltar à configuração anterior, use Desfazer integração.</p>`,
        confirmLabel: "Manter as duas",
        read: () => true,
      });
      if (ok) await runIntegration(btn, "Aplicando…", async () => reportResult(await integrationApi.apply(true), "Verificações combinadas."));
    } else if (action === "daemon-restart") {
      await runIntegration(btn, "Reiniciando…", async () => {
        await integrationApi.daemonRestart();
        showToast("Serviço em segundo plano reiniciado.", "ok");
      });
    } else if (action === "undo") {
      const ok = await openModal<boolean>({
        title: "Desfazer a integração com o Git?",
        body: `<p class="repos-modal-lead">O GitPass deixa de fornecer as credenciais do GitHub e de validar commits e envios. A configuração do Git que você tinha antes é restaurada e o serviço em segundo plano é removido.</p>
          <p class="repos-modal-lead muted">Suas contas e vínculos continuam salvos, e você pode ativar de novo quando quiser.</p>`,
        confirmLabel: "Desfazer integração",
        danger: true,
        read: () => true,
      });
      if (ok) await runIntegration(btn, "Desfazendo…", async () => reportResult(await integrationApi.undo(), "Integração desfeita."));
    }
  }

  // ---------- Modo emergência ----------

  function emergencyExplain(): string {
    return `
      <div class="int-em-explain">
        <p class="int-em-lead">Para quando o GitPass não consegue validar (por exemplo, o serviço não inicia) e você precisa salvar o trabalho.</p>
        <ul class="int-em-points">
          <li>${icon("check", 14)}<span>Libera só <strong>commits locais</strong>, por pouco tempo.</span></li>
          <li>${icon("lock", 14)}<span>O <strong>envio para o GitHub</strong> continua bloqueado.</span></li>
          <li>${icon("list", 14)}<span>Tudo fica registrado na auditoria abaixo.</span></li>
        </ul>
      </div>`;
  }

  function renderEmergency(): void {
    window.clearInterval(tick);
    tick = undefined;
    if (emergency.status === "loading") {
      emEl.innerHTML = `<div class="panel int-em"><span class="spinner" aria-hidden="true"></span><span class="muted">Carregando…</span></div>`;
      return;
    }
    if (emergency.status === "error") {
      emEl.innerHTML = `<div class="panel int-em">${emergencyExplain()}${errorView({
        title: "Não foi possível ler o modo emergência",
        error: emergency.error,
        actions: [{ label: "Tentar de novo", action: "em-retry", icon: "refresh" }],
        compact: true,
      })}</div>`;
      return;
    }
    const e = emergency.value;
    if (isEmergencyActive(e)) {
      const left = emergencyRemaining(e);
      emEl.innerHTML = `
        <div class="panel int-em int-em-active" role="status">
          <div class="int-em-clock">
            <span class="int-em-clock-icon" aria-hidden="true">${icon("timer", 22)}</span>
            <div>
              <p class="int-em-clock-label">Ativo · termina às ${esc(clockTime(e.until))}</p>
              ${left !== null ? `<p class="int-em-clock-value mono" data-ref="em-count" aria-hidden="true">${esc(formatCountdown(left))}</p>` : ""}
            </div>
          </div>
          <div class="int-em-active-text">
            <p class="int-em-sentence">${esc(emergencySentence(e))}</p>
            ${e.reason ? `<p class="int-em-meta"><span class="muted">Motivo:</span> ${esc(e.reason)}</p>` : ""}
            ${e.started_at ? `<p class="int-em-meta"><span class="muted">Ativado:</span> ${esc(fullDateTime(e.started_at))}</p>` : ""}
          </div>
          <button type="button" class="btn gp-btn-warning" data-action="em-stop">${icon("power", 14)}<span>Desativar agora</span></button>
        </div>`;
      if (left !== null) {
        tick = window.setInterval(() => {
          const el = emEl.querySelector<HTMLElement>('[data-ref="em-count"]');
          const ms = emergencyRemaining(e);
          if (el && ms !== null) el.textContent = formatCountdown(ms);
        }, 1000);
      }
      return;
    }
    const label = DURATIONS.find((d) => d.minutes === duration)?.label ?? `${duration} min`;
    emEl.innerHTML = `
      <div class="panel int-em">
        ${emergencyExplain()}
        <form class="int-em-form" data-ref="em-form" novalidate>
          <div class="int-em-field">
            <span class="field-label" id="int-em-dur-label">Duração</span>
            <div class="segmented" role="group" aria-labelledby="int-em-dur-label">
              ${DURATIONS.map((d) => `<button type="button" data-duration="${d.minutes}" aria-pressed="${d.minutes === duration}">${esc(d.label)}</button>`).join("")}
            </div>
          </div>
          <div class="int-em-field int-em-reason">
            <label class="field-label" for="int-em-reason">Motivo <span class="muted">(obrigatório)</span></label>
            <input class="input" id="int-em-reason" name="reason" maxlength="200" autocomplete="off" required aria-describedby="int-em-reason-hint" placeholder="Ex.: o serviço não inicia depois da atualização" value="${esc(reasonDraft)}" />
            <p class="int-em-hint muted" id="int-em-reason-hint">Aparece na auditoria. Não escreva senhas.</p>
          </div>
          <div class="int-em-submit">
            <button type="submit" class="btn gp-btn-warning">${icon("timer", 14)}<span>Ativar por ${esc(label)}</span></button>
          </div>
        </form>
      </div>`;
  }

  async function startEm(btn: HTMLButtonElement): Promise<void> {
    const input = emEl.querySelector<HTMLInputElement>("#int-em-reason");
    const reason = input?.value.trim() ?? "";
    const hint = emEl.querySelector<HTMLElement>("#int-em-reason-hint");
    if (!reason) {
      input?.setAttribute("aria-invalid", "true");
      if (hint) {
        hint.textContent = "Escreva o motivo para ativar o modo emergência.";
        hint.classList.add("err");
        hint.classList.remove("muted");
      }
      input?.focus();
      return;
    }
    try {
      await withBusy(btn, "Ativando…", () => startEmergency(duration, reason));
      reasonDraft = "";
      showToast("Modo emergência ativado. Commits locais liberados; envio continua bloqueado.", "ok");
      void loadAudit();
    } catch (e) {
      showToast(errMessage(e));
    }
  }

  // ---------- Auditoria ----------

  function renderAuditFilters(): void {
    const list = audit.status === "ok" ? audit.value : [];
    $("audit-filters").innerHTML = AUDIT_FILTERS.map((f) => {
      const n = f.kinds ? list.filter((e) => f.kinds!.includes(e.kind)).length : list.length;
      return `<button type="button" data-audit-filter="${f.id}" aria-pressed="${f.id === auditFilter}">${esc(f.label)}${
        audit.status === "ok" ? ` <span class="int-filter-count">${n}</span>` : ""
      }</button>`;
    }).join("");
  }

  function renderAudit(): void {
    renderAuditFilters();
    if (audit.status === "loading") {
      auditEl.innerHTML = `<div class="int-audit-list" aria-busy="true">${Array.from({ length: 4 }, () => `<div class="skeleton int-audit-sk"></div>`).join("")}</div>`;
      return;
    }
    if (audit.status === "error") {
      auditEl.innerHTML = errorView({
        title: "Não foi possível ler a auditoria",
        error: audit.error,
        actions: [{ label: "Tentar de novo", action: "refresh-audit", icon: "refresh" }],
      });
      return;
    }
    const f = AUDIT_FILTERS.find((x) => x.id === auditFilter)!;
    const rows = f.kinds ? audit.value.filter((e) => f.kinds!.includes(e.kind)) : audit.value;
    $("audit-count").textContent = `${rows.length} ${rows.length === 1 ? "evento" : "eventos"} na lista.`;
    if (!rows.length) {
      auditEl.innerHTML = `<div class="gp-group"><div class="empty-state int-audit-empty">
        <div class="empty-icon">${icon("list", 22)}</div>
        <h3>${audit.value.length ? "Nada com este filtro" : "Nenhum evento ainda"}</h3>
        <p>${audit.value.length ? "Escolha outro tipo acima." : "Quando o GitPass bloquear algo, avisar ou entrar no modo emergência, aparece aqui."}</p>
      </div></div>`;
      return;
    }
    auditEl.innerHTML = `<ol class="gp-group int-audit-list">${rows.map(auditRow).join("")}</ol>`;
  }

  async function loadAudit(): Promise<void> {
    try {
      audit = { status: "ok", value: await systemApi.audit(200) };
    } catch (error) {
      audit = { status: "error", error };
    }
    if (alive) renderAudit();
  }

  // ---------- Eventos ----------

  root.addEventListener("click", (ev) => {
    const t = ev.target as HTMLElement;
    const intBtn = t.closest<HTMLButtonElement>("button[data-int]");
    if (intBtn && !intBtn.disabled) {
      void onIntegrationAction(intBtn);
      return;
    }
    const dur = t.closest<HTMLButtonElement>("button[data-duration]");
    if (dur) {
      reasonDraft = emEl.querySelector<HTMLInputElement>("#int-em-reason")?.value ?? reasonDraft;
      duration = Number(dur.dataset.duration);
      renderEmergency();
      emEl.querySelector<HTMLElement>(`[data-duration="${duration}"]`)?.focus();
      return;
    }
    const filter = t.closest<HTMLButtonElement>("button[data-audit-filter]");
    if (filter) {
      auditFilter = filter.dataset.auditFilter as AuditFilter;
      renderAudit();
      root.querySelector<HTMLElement>(`[data-audit-filter="${auditFilter}"]`)?.focus();
      return;
    }
    const btn = t.closest<HTMLButtonElement>("button[data-action]");
    if (!btn || btn.disabled) return;
    switch (btn.dataset.action) {
      case "refresh-all":
        void withBusy(btn, "Verificando…", () => Promise.all([loadIntegration(), loadAudit()]).then(() => undefined));
        break;
      case "int-retry":
        intStatus = { status: "loading" };
        renderIntegration();
        void loadIntegration();
        break;
      case "refresh-audit":
        void withBusy(btn, "Atualizando…", loadAudit);
        break;
      case "em-retry":
        void refreshEmergency();
        break;
      case "em-stop":
        void withBusy(btn, "Desativando…", async () => {
          try {
            await stopEmergency();
            showToast("Modo emergência desativado. O GitPass volta a validar todos os commits.", "ok");
            void loadAudit();
          } catch (e) {
            showToast(errMessage(e));
          }
        });
        break;
    }
  });

  root.addEventListener("submit", (ev) => {
    const form = (ev.target as HTMLElement).closest<HTMLFormElement>('[data-ref="em-form"]');
    if (!form) return;
    ev.preventDefault();
    const btn = form.querySelector<HTMLButtonElement>('button[type="submit"]');
    if (btn && !btn.disabled) void startEm(btn);
  });

  root.addEventListener("input", (ev) => {
    const el = ev.target as HTMLInputElement;
    if (el.id !== "int-em-reason") return;
    reasonDraft = el.value;
    if (el.getAttribute("aria-invalid") === "true" && el.value.trim()) {
      el.removeAttribute("aria-invalid");
      const hint = emEl.querySelector<HTMLElement>("#int-em-reason-hint");
      if (hint) {
        hint.textContent = "Aparece na auditoria. Não escreva senhas.";
        hint.classList.remove("err");
        hint.classList.add("muted");
      }
    }
  });

  const offSystem = onSystemState((s) => {
    // Mudanças só do repo atual não redesenham o formulário (o motivo digitado e o foco ficam).
    if (s.emergency === emergency) return;
    const prevActive = emergency.status === "ok" && isEmergencyActive(emergency.value);
    emergency = s.emergency;
    const nowActive = emergency.status === "ok" && isEmergencyActive(emergency.value);
    if (!alive) return;
    renderEmergency();
    if (prevActive !== nowActive && audit.status !== "loading") void loadAudit();
  });

  let unlistenIntegration: (() => void) | null = null;
  void onIntegrationChanged((value) => {
    if (intBusy) return;
    intStatus = { status: "ok", value };
    renderIntegration();
  }).then((un) => {
    if (alive) unlistenIntegration = un;
    else un();
  });

  renderIntegration();
  renderAudit();
  void loadIntegration();
  void loadAudit();

  return () => {
    alive = false;
    window.clearInterval(tick);
    offSystem();
    unlistenIntegration?.();
  };
};
