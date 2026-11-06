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
  el.textContent = message;
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
