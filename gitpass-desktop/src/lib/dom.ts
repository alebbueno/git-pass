import { icon } from "./icons";

/** Seguro em texto e em valores de atributos entre aspas. */
export function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Encurta o diretório pessoal para `~`. */
export function shortenPath(p: string): string {
  return p
    .replace(/^\/Users\/[^/]+(?=\/|$)/, "~")
    .replace(/^\/home\/[^/]+(?=\/|$)/, "~")
    .replace(/^[A-Za-z]:\\Users\\[^\\]+(?=\\|$)/, "~");
}

export function errMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  return typeof e === "string" ? e : String(e);
}

/** Códigos de erro do GitPass (`GP-HOOK-012`, `GP-IPC-001`…), em qualquer ponto da mensagem. */
const GP_CODE_RE = /\[?\b(GP-[A-Z0-9]+(?:-[A-Z0-9]+)*)\b\]?/;

/** Separa o código `GP-*` do resto da mensagem (o texto fica sem o código e sem a pontuação que o acompanhava). */
export function splitGpCode(message: string): { code: string | null; text: string } {
  const m = GP_CODE_RE.exec(message);
  if (!m) return { code: null, text: message.trim() };
  const text = (message.slice(0, m.index) + message.slice(m.index + m[0].length))
    .replace(/^\s*[:·—–-]\s*/, "")
    .replace(/\s*[:·—–-]\s*$/, "")
    .replace(/\s{2,}/g, " ")
    .trim();
  return { code: m[1], text };
}

export function showToast(message: string, kind: "ok" | "err" = "err"): void {
  let container = document.getElementById("toast-container");
  if (!container) {
    container = document.createElement("div");
    container.id = "toast-container";
    container.className = "toast-container";
    container.setAttribute("role", "status");
    container.setAttribute("aria-live", "polite");
    document.body.appendChild(container);
  }
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.innerHTML = icon(kind === "ok" ? "check" : "alert");
  const { code, text } = kind === "err" ? splitGpCode(message) : { code: null, text: message };
  const body = document.createElement("span");
  body.className = "toast-text";
  body.textContent = text || "Algo deu errado.";
  el.appendChild(body);
  if (code) {
    const c = document.createElement("span");
    c.className = "toast-code";
    c.textContent = `Código ${code}`;
    el.appendChild(c);
  }
  container.appendChild(el);
  window.setTimeout(() => el.remove(), kind === "ok" ? 6000 : 12000);
}

/** Desativa o botão enquanto `task` corre; devolve o resultado ou lança o erro. */
export async function withBusy<T>(btn: HTMLButtonElement, busyLabel: string, task: () => Promise<T>): Promise<T> {
  const prev = btn.innerHTML;
  btn.disabled = true;
  btn.setAttribute("aria-busy", "true");
  btn.innerHTML = `<span class="spinner" aria-hidden="true"></span><span>${esc(busyLabel)}</span>`;
  try {
    return await task();
  } finally {
    btn.disabled = false;
    btn.removeAttribute("aria-busy");
    btn.innerHTML = prev;
  }
}

/**
 * Botões `[data-gp-copy="valor"]` em qualquer tela copiam o valor e mostram um visto por 2 s.
 * Instalado uma vez pelo shell.
 */
export function installCopyButtons(checkIcon: string): void {
  document.addEventListener("click", (ev) => {
    const btn = (ev.target as Element | null)?.closest?.<HTMLButtonElement>("button[data-gp-copy]");
    if (!btn) return;
    const value = btn.dataset.gpCopy ?? "";
    navigator.clipboard.writeText(value).then(
      () => {
        const prev = btn.innerHTML;
        const label = btn.getAttribute("aria-label");
        btn.innerHTML = checkIcon;
        btn.setAttribute("aria-label", "Copiado");
        btn.classList.add("is-copied");
        window.setTimeout(() => {
          if (!btn.isConnected) return;
          btn.innerHTML = prev;
          if (label) btn.setAttribute("aria-label", label);
          btn.classList.remove("is-copied");
        }, 2000);
      },
      (e) => showToast(`Não foi possível copiar: ${errMessage(e)}`)
    );
  });
}

export function relativeTime(iso: string | null): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const diff = (Date.now() - t) / 1000;
  const rtf = new Intl.RelativeTimeFormat("pt-BR", { numeric: "auto" });
  const steps: [number, Intl.RelativeTimeFormatUnit][] = [
    [60, "second"],
    [3600, "minute"],
    [86400, "hour"],
    [604800, "day"],
    [2629800, "week"],
    [31557600, "month"],
  ];
  let prevLimit = 1;
  for (const [limit, unit] of steps) {
    if (diff < limit) return rtf.format(-Math.floor(diff / prevLimit), unit);
    prevLimit = limit;
  }
  return rtf.format(-Math.floor(diff / 31557600), "year");
}

/** `15:42` (hoje) ou `6 nov., 15:42`. */
export function clockTime(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const sameDay = d.toDateString() === new Date().toDateString();
  return sameDay
    ? d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString("pt-BR", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

export function fullDateTime(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString("pt-BR", { dateStyle: "long", timeStyle: "short" });
}
