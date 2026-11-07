import type { EmergencyState } from "../lib/api-system";
import { clockTime, errMessage, esc, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { emergencyRemaining, formatCountdown, isEmergencyActive, onSystemState, stopEmergency } from "../lib/system-state";
import type { ViewContext } from "../lib/view";

/** Texto da faixa e da tela Integração: explícito sobre o que está liberado e o que não está. */
export function emergencySentence(e: EmergencyState): string {
  const until = clockTime(e.until);
  return until
    ? `Commits locais liberados até ${until}. Envio para o GitHub continua bloqueado.`
    : "Commits locais liberados. Envio para o GitHub continua bloqueado.";
}

/** Faixa global (vidro em tom de aviso) com contagem regressiva, motivo e "Desativar". */
export function mountEmergencyBanner(host: HTMLElement, ctx: ViewContext): void {
  let tick: number | undefined;
  let shown: EmergencyState | null = null;

  const updateCountdown = (): void => {
    if (!shown) return;
    const left = emergencyRemaining(shown);
    const el = host.querySelector<HTMLElement>("[data-ref='countdown']");
    if (el && left !== null) el.textContent = formatCountdown(left);
  };

  const render = (e: EmergencyState | null): void => {
    window.clearInterval(tick);
    tick = undefined;
    shown = e;
    document.documentElement.classList.toggle("gp-emergency-on", e !== null);
    if (!e) {
      host.hidden = true;
      host.innerHTML = "";
      return;
    }
    const left = emergencyRemaining(e);
    host.hidden = false;
    host.innerHTML = `
      <div class="gp-emergency-inner">
        <span class="gp-emergency-icon" aria-hidden="true">${icon("timer", 18)}</span>
        <div class="gp-emergency-text">
          <p class="gp-emergency-title">
            <strong>Modo emergência ativo</strong>
            ${left !== null ? `<span class="gp-emergency-count" aria-hidden="true">restam <span class="mono" data-ref="countdown">${esc(formatCountdown(left))}</span></span>` : ""}
          </p>
          <p class="gp-emergency-body">${esc(emergencySentence(e))}${e.reason ? ` <span class="gp-emergency-reason">Motivo: ${esc(e.reason)}</span>` : ""}</p>
        </div>
        <div class="gp-emergency-actions">
          <button type="button" class="btn ghost" data-em="details">Ver auditoria</button>
          <button type="button" class="btn gp-btn-warning" data-em="stop">${icon("power", 14)}<span>Desativar</span></button>
        </div>
      </div>`;
    if (left !== null) tick = window.setInterval(updateCountdown, 1000);
  };

  host.addEventListener("click", (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("button[data-em]");
    if (!btn || btn.disabled) return;
    if (btn.dataset.em === "details") {
      ctx.navigate("integration");
      return;
    }
    void withBusy(btn, "Desativando…", async () => {
      try {
        await stopEmergency();
        showToast("Modo emergência desativado. O GitPass volta a validar todos os commits.", "ok");
      } catch (e) {
        showToast(errMessage(e));
      }
    });
  });

  onSystemState((s) => {
    const e = s.emergency.status === "ok" && isEmergencyActive(s.emergency.value) ? s.emergency.value : null;
    if (e === shown || (e && shown && e.until === shown.until && e.reason === shown.reason)) return;
    render(e);
  });
}
