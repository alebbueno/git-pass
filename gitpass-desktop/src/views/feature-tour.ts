import "./feature-tour.css";
import { esc } from "../lib/dom";
import { FEATURE_TOUR_CHANGED_EVENT, FEATURE_TOUR_REQUEST_EVENT, setFeatureTour } from "../lib/feature-tour";
import { icon, type IconName } from "../lib/icons";
import type { Route, ViewContext } from "../lib/view";

type TourStep = {
  id: string;
  eyebrow: string;
  title: string;
  body: string;
  icon: IconName;
  tips: string[];
  /** Abre a área relacionada e fecha o tour. */
  go?: { label: string; route: Route };
};

const STEPS: TourStep[] = [
  {
    id: "intro",
    eyebrow: "Visão geral",
    title: "Uma conta certa em cada projeto",
    body: "O GitPass liga pastas locais a perfis do GitHub, verifica commits no terminal e avisa quando algo foge da identidade esperada.",
    icon: "logo",
    tips: ["Vários perfis no mesmo Mac, sem misturar autor de commit.", "O Painel resume o que precisa de atenção."],
  },
  {
    id: "overview",
    eyebrow: "Painel",
    title: "Resumo e atalhos",
    body: "No Painel você vê contas, vínculos, commits para enviar e o repositório em foco. É o ponto de partida do dia a dia.",
    icon: "dashboard",
    tips: ["Repositórios recentes abrem a gaveta de detalhes.", "Alertas levam direto à auditoria da integração."],
    go: { label: "Abrir Painel", route: "overview" },
  },
  {
    id: "repos",
    eyebrow: "Repositórios",
    title: "Vínculos, radar e identidade",
    body: "Cada pasta vinculada usa um perfil. Filtros e chips mostram pendências do Git; na gaveta você revisa identidade, push e histórico do GitPass.",
    icon: "repo",
    tips: ["Filtro «Com pendências» usa o radar de trabalho.", "Corrija identidade antes de commitar, se o diagnóstico avisar."],
    go: { label: "Ver repositórios", route: "repos" },
  },
  {
    id: "integration",
    eyebrow: "Integração e diagnóstico",
    title: "Git seguro no terminal",
    body: "A integração instala credenciais, hooks e o serviço em segundo plano. O Diagnóstico explica bloqueios e o modo emergência.",
    icon: "stethoscope",
    tips: ["Repita «Aplicar» na integração após atualizar o app.", "Auditoria lista commits bloqueados ou não confirmados."],
    go: { label: "Abrir integração", route: "integration" },
  },
  {
    id: "rules",
    eyebrow: "Configurações",
    title: "Regras de conta e histórico",
    body: "Regras sugerem um perfil ao mapear pastas ou repositórios de uma organização. Alterações de vínculo ficam no histórico, com opção de desfazer.",
    icon: "settings",
    tips: ["Regras não mudam vínculos já existentes sozinhas.", "Desfazer restaura conta ou identidade do snapshot."],
    go: { label: "Abrir configurações", route: "settings" },
  },
  {
    id: "palette",
    eyebrow: "Paleta",
    title: "Atalho ⌘K (Ctrl+K)",
    body: "A paleta de comandos junta navegação, revisão de identidade e ações frequentes. Funciona em qualquer tela.",
    icon: "command",
    tips: ["Procure por nome do projeto ou do perfil.", "Enter executa; Esc fecha."],
  },
  {
    id: "cloud",
    eyebrow: "Na nuvem",
    title: "Pull requests na fila",
    body: "Na aba Pull requests você acompanha PRs abertos nos repositórios vinculados, junto com a lista de repositórios na nuvem.",
    icon: "pullRequest",
    tips: ["Útil para revisar o que está em aberto antes do merge.", "Requer conta conectada com acesso à API."],
    go: { label: "Abrir Na nuvem", route: "cloud" },
  },
  {
    id: "done",
    eyebrow: "Pronto",
    title: "Explore no seu ritmo",
    body: "Você pode reabrir este tour em Configurações ou no Painel. O guia de primeira configuração continua disponível se ainda não terminou os passos iniciais.",
    icon: "checkCircle",
    tips: ["Conecte contas em Contas.", "Mapeie pastas em Repositórios → Mapear pasta."],
  },
];

let openSeq = 0;

function renderStep(step: TourStep, index: number): string {
  const tips = step.tips.map((t) => `<li>${esc(t)}</li>`).join("");
  const go =
    step.go ?
      `<button type="button" class="control ghost" data-tour-go="${esc(step.go.route)}">${esc(step.go.label)}${icon("chevronRight")}</button>`
    : "";
  return `
    <div class="feature-tour-head">
      <div>
        <p class="feature-tour-kicker">${esc(step.eyebrow)}</p>
        <h2 class="feature-tour-title" id="feature-tour-title" tabindex="-1">${esc(step.title)}</h2>
      </div>
      <button type="button" class="icon-button ghost" data-tour-action="close" aria-label="Fechar tour">${icon("x")}</button>
    </div>
    <article class="feature-tour-card" aria-labelledby="feature-tour-title">
      <div class="feature-tour-icon" aria-hidden="true">${icon(step.icon, 22)}</div>
      <div>
        <p class="feature-tour-body">${esc(step.body)}</p>
        <ul class="feature-tour-tips">${tips}</ul>
        ${go ? `<div class="row-actions" style="margin-top: var(--sp-3)">${go}</div>` : ""}
      </div>
    </article>
    <div class="feature-tour-progress">
      <div class="feature-tour-dots" role="tablist" aria-label="Passos do tour">
        ${STEPS.map(
          (s, i) =>
            `<button type="button" class="feature-tour-dot" role="tab" data-tour-index="${i}" aria-label="${esc(s.title)}"${
              i === index ? ' aria-current="step"' : ""
            }"></button>`
        ).join("")}
      </div>
      <span class="feature-tour-step-label">${index + 1} de ${STEPS.length}</span>
    </div>
    <footer class="feature-tour-foot">
      <button type="button" class="control ghost" data-tour-action="skip">Pular tour</button>
      <div class="feature-tour-foot-actions">
        <button type="button" class="control" data-tour-action="prev"${index === 0 ? " disabled" : ""}>${icon("chevronLeft")}Anterior</button>
        <button type="button" class="control primary" data-tour-action="${index === STEPS.length - 1 ? "finish" : "next"}">
          ${index === STEPS.length - 1 ? "Concluir" : "Próximo"}${icon("chevronRight")}
        </button>
      </div>
    </footer>`;
}

function openTour(ctx: ViewContext): void {
  const seq = ++openSeq;
  let index = 0;
  const previousFocus = document.activeElement as HTMLElement | null;

  const backdrop = document.createElement("div");
  backdrop.className = "feature-tour-backdrop modal-backdrop";
  backdrop.innerHTML = `<div class="feature-tour" role="dialog" aria-modal="true" aria-labelledby="feature-tour-title"></div>`;
  const dialog = backdrop.querySelector<HTMLElement>(".feature-tour")!;

  const close = (kind: "done" | "skipped"): void => {
    if (seq !== openSeq) return;
    openSeq++;
    setFeatureTour(kind);
    document.dispatchEvent(new CustomEvent(FEATURE_TOUR_CHANGED_EVENT));
    backdrop.remove();
    document.removeEventListener("keydown", onKey);
    previousFocus?.focus?.({ preventScroll: true });
  };

  const paint = (): void => {
    dialog.innerHTML = renderStep(STEPS[index], index);
    const title = dialog.querySelector<HTMLElement>("#feature-tour-title");
    title?.focus({ preventScroll: true });
  };

  const onKey = (e: KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      close("skipped");
      return;
    }
    if (e.key === "ArrowRight" && !e.metaKey && !e.ctrlKey && !e.altKey) {
      if (index < STEPS.length - 1) {
        index++;
        paint();
      }
    }
    if (e.key === "ArrowLeft" && !e.metaKey && !e.ctrlKey && !e.altKey) {
      if (index > 0) {
        index--;
        paint();
      }
    }
  };

  const onClick = (ev: Event): void => {
    const t = ev.target as HTMLElement;
    if (t === backdrop) {
      close("skipped");
      return;
    }
    const go = t.closest<HTMLElement>("[data-tour-go]");
    if (go?.dataset.tourGo) {
      close("done");
      ctx.navigate(go.dataset.tourGo as Route);
      return;
    }
    const dot = t.closest<HTMLButtonElement>("[data-tour-index]");
    if (dot?.dataset.tourIndex) {
      index = Number(dot.dataset.tourIndex);
      paint();
      return;
    }
    const act = t.closest<HTMLButtonElement>("[data-tour-action]");
    if (!act) return;
    switch (act.dataset.tourAction) {
      case "close":
      case "skip":
        close("skipped");
        break;
      case "prev":
        if (index > 0) {
          index--;
          paint();
        }
        break;
      case "next":
        if (index < STEPS.length - 1) {
          index++;
          paint();
        }
        break;
      case "finish":
        close("done");
        break;
    }
  };

  backdrop.addEventListener("click", onClick);
  document.addEventListener("keydown", onKey);
  document.body.appendChild(backdrop);
  paint();

  if (seq !== openSeq) backdrop.remove();
}

export function installFeatureTour(ctx: ViewContext): void {
  document.addEventListener(FEATURE_TOUR_REQUEST_EVENT, () => openTour(ctx));
}
