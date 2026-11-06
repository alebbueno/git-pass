import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type CheckState = "ok" | "missing" | "outdated" | "conflict" | "error";

export type ComponentStatus = {
  state: CheckState;
  summary: string;
  /** Valor configurado quando não é do GitPass (conflito). */
  current_value: string | null;
  /** Valor que o GitPass substituiu ou encadeia (guardado na cópia de segurança). */
  previous_value: string | null;
};

export type IntegrationStatus = {
  /** Falso depois de "Desfazer integração", até o utilizador escolher Reparar. */
  enabled: boolean;
  git_found: boolean;
  binaries: ComponentStatus;
  credentials: ComponentStatus;
  hooks: ComponentStatus;
  daemon: ComponentStatus;
  has_backup: boolean;
  ready: boolean;
};

export type IntegrationReport = {
  status: IntegrationStatus;
  changes: string[];
  warnings: string[];
};

/** Emitido pelo backend com um `IntegrationStatus` sempre que a integração muda (incluindo no arranque). */
export const INTEGRATION_CHANGED_EVENT = "gitpass-integration-changed";

export const integrationApi = {
  status: () => invoke<IntegrationStatus>("integration_status"),
  /** Instala ou repara. `takeOverHooks` substitui um `core.hooksPath` próprio e encadeia-o. */
  apply: (takeOverHooks = false) => invoke<IntegrationReport>("integration_apply", { takeOverHooks }),
  undo: () => invoke<IntegrationReport>("integration_undo"),
  daemonRestart: () => invoke<IntegrationStatus>("integration_daemon_restart"),
  daemonStop: () => invoke<IntegrationStatus>("integration_daemon_stop"),
};

export function onIntegrationChanged(cb: (status: IntegrationStatus) => void): Promise<UnlistenFn> {
  return listen<IntegrationStatus>(INTEGRATION_CHANGED_EVENT, (ev) => cb(ev.payload));
}

/** Resumo de uma linha para mosaicos e barras de estado. */
export function integrationHeadline(s: IntegrationStatus): { kind: "success" | "warning" | "error"; label: string; detail: string } {
  if (!s.enabled) return { kind: "warning", label: "Desativada", detail: "O Git não usa o GitPass" };
  if (s.ready) return { kind: "success", label: "Ativa", detail: "Credenciais, hooks e serviço prontos" };
  const parts = [s.credentials, s.hooks, s.daemon];
  if (parts.some((c) => c.state === "error" || c.state === "missing") || s.binaries.state === "error") {
    return { kind: "error", label: "Reparar", detail: "Falta configurar uma peça" };
  }
  return { kind: "warning", label: "Atenção", detail: "Há uma peça por rever" };
}
