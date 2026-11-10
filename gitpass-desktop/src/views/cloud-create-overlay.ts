import "./cloud-create-overlay.css";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { animate, createTimeline } from "animejs";
import { CLONE_PROGRESS_EVENT, type CloneProgressPayload } from "../lib/api-cloud";
import { esc, shortenPath } from "../lib/dom";
import { icon } from "../lib/icons";

const MIN_WORKING_MS = 1500;
const SUCCESS_HOLD_MS = 3200;

const STEP_PHASES: Record<string, number> = {
  creating: 0,
  starting: 1,
  counting: 1,
  compressing: 1,
  receiving: 1,
  resolving: 1,
  checkout: 1,
  binding: 2,
};

const PHASE_SPAN: Record<string, [number, number]> = {
  starting: [0, 0],
  creating: [2, 12],
  counting: [12, 18],
  compressing: [18, 24],
  receiving: [24, 78],
  resolving: [78, 92],
  checkout: [92, 98],
  binding: [98, 100],
};

const PHASE_LABEL: Record<string, string> = {
  starting: "Preparando…",
  creating: "Criando no GitHub…",
  counting: "Clonando · contando objetos",
  compressing: "Clonando · comprimindo",
  receiving: "Clonando · recebendo dados",
  resolving: "Clonando · resolvendo deltas",
  checkout: "Clonando · extraindo arquivos",
  binding: "Vinculando à conta…",
};

function phasePercent(phase: string, percent: number): number {
  const [from, to] = PHASE_SPAN[phase] ?? [0, 100];
  const p = Math.max(0, Math.min(100, percent));
  return Math.round(from + ((to - from) * p) / 100);
}

const reducedMotion = (): boolean =>
  typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

export type CloudCreateOverlayHandle = {
  setPhase: (phase: string, percent: number) => void;
  waitMinimumTime: (startedAt: number) => Promise<void>;
  showSuccess: (path: string) => Promise<void>;
  showError: (message: string) => Promise<void>;
  close: () => void;
};

export function openCloudCreateOverlay(opts: {
  fullName: string;
  destLabel: string;
}): CloudCreateOverlayHandle {
  const backdrop = document.createElement("div");
  backdrop.className = "cloud-create-flow-backdrop";
  backdrop.setAttribute("role", "presentation");
  backdrop.innerHTML = `
    <div class="cloud-create-flow" role="dialog" aria-modal="true" aria-labelledby="cloud-create-flow-title" data-mode="working">
      <div class="cloud-create-flow-inner">
        <div class="cloud-create-flow-working">
          <div class="cloud-create-flow-head">
            <div class="cloud-create-flow-orb" aria-hidden="true">${icon("repo", 26)}</div>
            <h2 class="cloud-create-flow-title" id="cloud-create-flow-title">Criando repositório</h2>
            <p class="cloud-create-flow-sub"><code>${esc(opts.fullName)}</code><br/>${esc(opts.destLabel)}</p>
          </div>
          <ol class="cloud-create-flow-steps" aria-label="Etapas">
            <li class="cloud-create-flow-step" data-step="0" data-state="active"><span class="step-dot">1</span><span>Criar no GitHub</span></li>
            <li class="cloud-create-flow-step" data-step="1" data-state=""><span class="step-dot">2</span><span>Clonar na pasta</span></li>
            <li class="cloud-create-flow-step" data-step="2" data-state=""><span class="step-dot">3</span><span>Vincular ao GitPass</span></li>
          </ol>
          <div class="cloud-create-flow-progress">
            <div class="cloud-create-flow-progress-track" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0" aria-label="Progresso">
              <span class="cloud-create-flow-progress-fill"></span>
            </div>
            <div class="cloud-create-flow-progress-label">
              <span data-flow-phase>Preparando…</span>
              <span data-flow-pct>0%</span>
            </div>
          </div>
        </div>
        <div class="cloud-create-flow-success">
          <div class="cloud-create-flow-check" aria-hidden="true">${icon("checkCircle", 32)}</div>
          <h2 class="cloud-create-flow-title">Repositório criado</h2>
          <p class="caption muted">Projeto pronto na pasta abaixo, já vinculado à conta escolhida.</p>
          <div class="cloud-create-flow-path">${icon("folder")}<code class="truncate" data-success-path></code></div>
          <div class="cloud-create-flow-actions">
            <button type="button" class="control primary" data-flow-done>Continuar</button>
          </div>
        </div>
        <div class="cloud-create-flow-error">
          <span class="state error">${icon("xCircle")}<span data-flow-err></span></span>
          <button type="button" class="control" data-flow-done>Fechar</button>
        </div>
      </div>
    </div>
  `;

  const flow = backdrop.querySelector<HTMLElement>(".cloud-create-flow")!;
  const fill = backdrop.querySelector<HTMLElement>(".cloud-create-flow-progress-fill")!;
  const bar = backdrop.querySelector<HTMLElement>(".cloud-create-flow-progress-track")!;
  const phaseEl = backdrop.querySelector<HTMLElement>("[data-flow-phase]")!;
  const pctEl = backdrop.querySelector<HTMLElement>("[data-flow-pct]")!;
  const steps = [...backdrop.querySelectorAll<HTMLElement>(".cloud-create-flow-step")];
  const successPath = backdrop.querySelector<HTMLElement>("[data-success-path]")!;
  const errEl = backdrop.querySelector<HTMLElement>("[data-flow-err]")!;
  const orb = flow.querySelector<HTMLElement>(".cloud-create-flow-orb")!;

  let closed = false;
  let unlisten: UnlistenFn | null = null;
  let settle: (() => void) | null = null;
  let successTimer = 0;

  const trapFocus = (ev: KeyboardEvent): void => {
    if (ev.key === "Escape") ev.preventDefault();
  };

  const finish = (): void => {
    settle?.();
    settle = null;
    handle.close();
  };

  backdrop.addEventListener("click", (ev) => {
    if (ev.target === backdrop && flow.dataset.mode === "success") finish();
  });
  backdrop.querySelectorAll("[data-flow-done]").forEach((b) => b.addEventListener("click", finish));
  document.addEventListener("keydown", trapFocus);

  document.body.appendChild(backdrop);

  if (!reducedMotion()) {
    animate(backdrop, { opacity: [0, 1], duration: 280, ease: "out(3)" });
    animate(flow, {
      opacity: [0, 1],
      translateY: [20, 0],
      scale: [0.96, 1],
      duration: 520,
      ease: "out(4)",
    });
    animate(orb, {
      rotate: [0, 5, -3, 0],
      scale: [1, 1.05, 1],
      duration: 2200,
      ease: "inOut(2)",
      loop: true,
    });
  } else {
    backdrop.style.opacity = "1";
    flow.style.opacity = "1";
    flow.style.transform = "none";
  }

  const handle: CloudCreateOverlayHandle = {
    setPhase(phase, percent) {
      const pct = phasePercent(phase, percent);
      fill.style.width = `${pct}%`;
      bar.setAttribute("aria-valuenow", String(pct));
      phaseEl.textContent = PHASE_LABEL[phase] ?? "Trabalhando…";
      pctEl.textContent = `${pct}%`;
      const stepIdx = STEP_PHASES[phase] ?? 0;
      steps.forEach((el, i) => {
        if (i < stepIdx) el.dataset.state = "done";
        else if (i === stepIdx) el.dataset.state = "active";
        else el.dataset.state = "";
      });
    },

    async waitMinimumTime(startedAt) {
      const elapsed = Date.now() - startedAt;
      const wait = Math.max(0, MIN_WORKING_MS - elapsed);
      if (wait > 0) await new Promise((r) => window.setTimeout(r, wait));
    },

    showSuccess(path) {
      flow.dataset.mode = "success";
      successPath.textContent = shortenPath(path);
      successPath.title = path;
      if (!reducedMotion()) {
        const successBlock = flow.querySelector(".cloud-create-flow-success")!;
        const check = flow.querySelector(".cloud-create-flow-check")!;
        const tl = createTimeline({ defaults: { ease: "out(4)" } });
        tl.add(successBlock, { opacity: [0, 1], translateY: [14, 0], duration: 420 });
        tl.add(check, { scale: [0.45, 1.1, 1], rotate: [-10, 0], duration: 580 }, "-=260");
        tl.add(
          flow.querySelectorAll(".cloud-create-flow-path, .cloud-create-flow-actions > *"),
          { opacity: [0, 1], translateY: [10, 0], duration: 340, delay: (_, i) => i * 80 },
          "-=220"
        );
      }
      return new Promise<void>((resolve) => {
        settle = resolve;
        successTimer = window.setTimeout(() => {
          if (!closed && flow.dataset.mode === "success") finish();
        }, SUCCESS_HOLD_MS);
      });
    },

    showError(message) {
      flow.dataset.mode = "error";
      errEl.textContent = message;
      if (!reducedMotion()) {
        animate(flow.querySelector(".cloud-create-flow-error")!, {
          opacity: [0, 1],
          translateY: [12, 0],
          duration: 360,
          ease: "out(3)",
        });
      }
      return new Promise<void>((resolve) => {
        settle = resolve;
      });
    },

    close() {
      if (closed) return;
      closed = true;
      window.clearTimeout(successTimer);
      document.removeEventListener("keydown", trapFocus);
      unlisten?.();
      const done = () => backdrop.remove();
      if (!reducedMotion()) {
        animate(backdrop, {
          opacity: [1, 0],
          duration: 220,
          ease: "in(2)",
          onComplete: done,
        });
      } else {
        done();
      }
    },
  };

  void listen<CloneProgressPayload>(CLONE_PROGRESS_EVENT, (ev) => {
    if (ev.payload.full_name.toLowerCase() !== opts.fullName.toLowerCase()) return;
    handle.setPhase(ev.payload.phase, ev.payload.percent);
  }).then((u) => {
    if (closed) u();
    else unlisten = u;
  });

  handle.setPhase("creating", 0);
  return handle;
}
