import {
  systemApi,
  type AuditKind,
  type CurrentRepo,
  type CurrentRepoSource,
  type CurrentRepoState,
  type Diagnostics,
  type EmergencyState,
  type ValidationVerdict,
} from "./api-system";
import type { IconName } from "./icons";

/**
 * Estado do sistema partilhado pelo shell e pelas telas: repo atual (fixado > foco > recente)
 * e modo emergência. O backend avisa por evento; aqui só guardamos a última resposta.
 */

export type Loadable<T> = { status: "loading" } | { status: "ok"; value: T } | { status: "error"; error: unknown };

export type SystemSnapshot = {
  current: Loadable<CurrentRepoState>;
  emergency: Loadable<EmergencyState>;
};

let snap: SystemSnapshot = { current: { status: "loading" }, emergency: { status: "loading" } };
const subs = new Set<(s: SystemSnapshot) => void>();
let expiryTimer: number | undefined;

export function systemSnapshot(): SystemSnapshot {
  return snap;
}

/** Chama `cb` já com o estado atual; devolve a função para deixar de ouvir. */
export function onSystemState(cb: (s: SystemSnapshot) => void): () => void {
  subs.add(cb);
  cb(snap);
  return () => subs.delete(cb);
}

function publish(next: Partial<SystemSnapshot>): void {
  snap = { ...snap, ...next };
  scheduleExpiryCheck();
  for (const cb of subs) cb(snap);
}

export async function refreshCurrentRepo(): Promise<void> {
  try {
    publish({ current: { status: "ok", value: await systemApi.currentRepo() } });
  } catch (error) {
    publish({ current: { status: "error", error } });
  }
}

export async function refreshEmergency(): Promise<void> {
  try {
    publish({ emergency: { status: "ok", value: await systemApi.emergencyState() } });
  } catch (error) {
    publish({ emergency: { status: "error", error } });
  }
}

/** `null` desafixa. Lança o erro para a tela mostrar. */
export async function pinRepo(bindingId: string | null): Promise<CurrentRepoState> {
  const value = await systemApi.pinRepo(bindingId);
  publish({ current: { status: "ok", value } });
  return value;
}

export async function startEmergency(minutes: number, reason: string): Promise<EmergencyState> {
  const value = await systemApi.emergencyStart(minutes, reason);
  publish({ emergency: { status: "ok", value } });
  return value;
}

export async function stopEmergency(): Promise<EmergencyState> {
  const value = await systemApi.emergencyStop();
  publish({ emergency: { status: "ok", value } });
  return value;
}

export function currentState(): CurrentRepoState | null {
  return snap.current.status === "ok" ? snap.current.value : null;
}

export function activeEmergency(): EmergencyState | null {
  const e = snap.emergency.status === "ok" ? snap.emergency.value : null;
  return e && isEmergencyActive(e) ? e : null;
}

export function isEmergencyActive(e: EmergencyState): boolean {
  if (!e.active) return false;
  if (!e.until) return true;
  const t = Date.parse(e.until);
  return Number.isNaN(t) || t > Date.now();
}

/** Milissegundos até o fim; `null` sem prazo conhecido. */
export function emergencyRemaining(e: EmergencyState): number | null {
  if (!e.until) return null;
  const t = Date.parse(e.until);
  return Number.isNaN(t) ? null : Math.max(0, t - Date.now());
}

/** `23:41` ou `1:05:09`. */
export function formatCountdown(ms: number): string {
  const total = Math.ceil(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(h > 0 ? 2 : 1, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** Quando o prazo passa, o backend já terá desligado; buscamos o estado novo (e a auditoria registra a expiração). */
function scheduleExpiryCheck(): void {
  window.clearTimeout(expiryTimer);
  const e = snap.emergency.status === "ok" ? snap.emergency.value : null;
  if (!e?.active) return;
  const left = emergencyRemaining(e);
  if (left === null) return;
  expiryTimer = window.setTimeout(() => void refreshEmergency(), Math.min(left + 1500, 2_147_000_000));
}

let started = false;

export function initSystemState(): void {
  if (started) return;
  started = true;
  void systemApi.onCurrentRepoChanged((value) => publish({ current: { status: "ok", value } })).catch(() => undefined);
  void systemApi.onEmergencyChanged((value) => publish({ emergency: { status: "ok", value } })).catch(() => undefined);
  void refreshCurrentRepo();
  void refreshEmergency();
}

// ---------- Diagnóstico (contador da barra lateral) ----------

export const DIAGNOSTICS_UPDATED_EVENT = "gitpass:diagnostics-updated";

/** A tela Diagnóstico avisa o shell para atualizar o contador sem pedir de novo. */
export function announceDiagnostics(d: Diagnostics): void {
  document.dispatchEvent(new CustomEvent<Diagnostics>(DIAGNOSTICS_UPDATED_EVENT, { detail: d }));
}

// ---------- Rótulos ----------

export const SOURCE_META: Record<CurrentRepoSource, { icon: IconName; label: string; detail: string }> = {
  pinned: { icon: "pin", label: "Fixado", detail: "Você fixou este repositório. Ele continua como atual até você desafixar." },
  focus: { icon: "target", label: "Em foco", detail: "É o repositório aberto no app em primeiro plano (VS Code, Terminal…)." },
  recent: { icon: "clock", label: "Recente", detail: "Foi o último repositório que o Git usou com o GitPass." },
};

export type VerdictMeta = { kind: "success" | "warning" | "error" | "neutral"; label: string; detail: string };

export function verdictMeta(v: ValidationVerdict | null): VerdictMeta {
  switch (v) {
    case "ok":
      return { kind: "success", label: "OK", detail: "A conta e o endereço do GitHub conferem." };
    case "blocked":
      return { kind: "error", label: "Bloqueado", detail: "O GitPass bloqueou o último commit ou envio." };
    case "unconfirmed":
      return {
        kind: "warning",
        label: "Não confirmado",
        detail: "Permitido com aviso: não deu para confirmar no GitHub (sem conexão ou API indisponível).",
      };
    default:
      return { kind: "neutral", label: "Sem verificação", detail: "O Git ainda não passou pelo GitPass neste repositório." };
  }
}

export function currentRepoName(c: Pick<CurrentRepo, "full_name" | "root_path">): string {
  if (c.full_name) return c.full_name;
  const parts = c.root_path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? c.root_path;
}

export const AUDIT_META: Record<AuditKind, { label: string; kind: "success" | "warning" | "error" | "info"; icon: IconName }> = {
  blocked: { label: "Bloqueado", kind: "error", icon: "x" },
  unconfirmed: { label: "Não confirmado", kind: "warning", icon: "alert" },
  emergency_on: { label: "Emergência ativada", kind: "warning", icon: "timer" },
  emergency_off: { label: "Emergência desativada", kind: "info", icon: "power" },
  emergency_expired: { label: "Emergência expirou", kind: "info", icon: "clock" },
  commit_allowed_emergency: { label: "Commit na emergência", kind: "warning", icon: "commit" },
  ci_bypass: { label: "Liberado no CI", kind: "info", icon: "shield" },
};
