import { errMessage, esc, splitGpCode } from "./dom";
import { icon, type IconName } from "./icons";

/**
 * Componente único de erro: título simples, causa, ação sugerida e, em "Detalhes",
 * o código `GP-*` (copiável) com a mensagem original. Tudo que vem do backend passa por `esc`.
 */

export type ParsedError = {
  code: string | null;
  /** Texto para o usuário, sem o código. */
  cause: string;
  /** Mensagem original, para "Detalhes". */
  raw: string;
  /** O backend desta versão não tem o comando (ex.: recurso ainda não disponível). */
  unavailable: boolean;
};

const UNAVAILABLE_RE = /\bcommand\s+\S+\s+not\s+found\b|\bunknown command\b|comando \S+ indispon/i;

export function parseError(e: unknown): ParsedError {
  const raw = errMessage(e).trim();
  const unavailable = UNAVAILABLE_RE.test(raw);
  const { code, text } = splitGpCode(raw);
  const cause = unavailable
    ? "Esta versão do GitPass ainda não oferece este recurso. Atualize o app e tente de novo."
    : text || "Algo deu errado.";
  return { code, cause, raw, unavailable };
}

export type ErrorAction = {
  label: string;
  /** Vai para `data-action` no botão; a tela trata o clique. */
  action: string;
  icon?: IconName;
  primary?: boolean;
  /** Atributos `data-*` extras (ex.: `{ retry: id }` → `data-retry="…"`). */
  data?: Record<string, string>;
};

export type ErrorViewOptions = {
  title: string;
  error: unknown;
  actions?: ErrorAction[];
  /** Variante em linha (dentro de um painel), sem moldura própria. */
  compact?: boolean;
  /** `warning` para falhas que não impedem o resto da tela. */
  tone?: "error" | "warning";
};

export function codeChip(code: string): string {
  return `<span class="gp-code"><span class="gp-code-text">${esc(code)}</span><button type="button" class="gp-code-copy" data-gp-copy="${esc(
    code
  )}" aria-label="Copiar o código ${esc(code)}" title="Copiar o código">${icon("copy", 12)}</button></span>`;
}

export function errorView(o: ErrorViewOptions): string {
  const p = parseError(o.error);
  const tone = o.tone ?? "error";
  const actions = (o.actions ?? [])
    .map(
      (a) =>
        `<button type="button" class="btn${a.primary ? " primary" : ""}" data-action="${esc(a.action)}"${Object.entries(a.data ?? {})
          .map(([k, v]) => ` data-${esc(k)}="${esc(v)}"`)
          .join("")}>${a.icon ? icon(a.icon, 14) : ""}<span>${esc(a.label)}</span></button>`
    )
    .join("");
  const showDetails = p.code !== null || p.raw !== p.cause;
  const details = showDetails
    ? `<details class="gp-error-details">
        <summary>Detalhes</summary>
        <div class="gp-error-details-body">
          ${p.code ? `<div class="gp-error-code-row"><span class="muted">Código</span>${codeChip(p.code)}</div>` : ""}
          ${p.raw && p.raw !== p.cause ? `<p class="gp-error-raw mono">${esc(p.raw)}</p>` : ""}
        </div>
      </details>`
    : "";
  return `
    <div class="gp-error gp-error-${tone}${o.compact ? " gp-error-compact" : ""}" role="${tone === "error" ? "alert" : "status"}">
      <span class="gp-error-icon" aria-hidden="true">${icon(tone === "error" ? "xCircle" : "alert", 18)}</span>
      <div class="gp-error-body">
        <p class="gp-error-title">${esc(o.title)}</p>
        <p class="gp-error-cause">${esc(p.cause)}</p>
        ${actions ? `<div class="gp-error-actions">${actions}</div>` : ""}
        ${details}
      </div>
    </div>`;
}
