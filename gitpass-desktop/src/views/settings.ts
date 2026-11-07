import "./settings.css";
import { api, type OauthConfigStatus } from "../lib/api";
import { integrationApi, integrationHeadline, onIntegrationChanged, type IntegrationStatus } from "../lib/api-integration";
import { errMessage, esc, showToast, withBusy } from "../lib/dom";
import { errorView } from "../lib/errors";
import { brandSymbol, icon } from "../lib/icons";
import { reducedTransparency, setReducedTransparency, setTheme, storedTheme, THEME_CHANGED_EVENT, type ThemeMode } from "../lib/theme";
import { DATA_CHANGED_EVENT, type MountView } from "../lib/view";

const THEME_OPTIONS: { mode: ThemeMode; label: string; icon: "monitor" | "sun" | "moon" }[] = [
  { mode: "system", label: "Sistema", icon: "monitor" },
  { mode: "light", label: "Claro", icon: "sun" },
  { mode: "dark", label: "Escuro", icon: "moon" },
];

function pathBox(value: string, what: string): string {
  return `<div class="settings-path"><code class="settings-path-text">${esc(value)}</code>
    <button type="button" class="icon-button" data-gp-copy="${esc(value)}" aria-label="Copiar ${esc(what)}" title="Copiar ${esc(what)}">${icon("copy")}</button></div>`;
}

const loadingState = `<span class="state neutral"><span class="spinner" aria-hidden="true"></span>Verificando…</span>`;

export const mount: MountView = (root, ctx) => {
  let alive = true;

  root.innerHTML = `
    <div class="page-head">
      <div class="heading-group"><h1>Configurações</h1><p>Personalize a aparência e acompanhe a integração com o Git.</p></div>
    </div>

    <div class="settings">
      <section class="surface" aria-labelledby="settings-h-appearance">
        <h2 class="section-title" id="settings-h-appearance">Aparência</h2>
        <div class="settings-row">
          <div><strong>Tema</strong><p class="caption muted">Escolha como o GitPass aparece neste dispositivo.</p></div>
          <div class="theme-options" role="group" aria-label="Tema">${THEME_OPTIONS.map(
            (o) => `<button type="button" class="control" data-theme-mode="${o.mode}" aria-pressed="false">${icon(o.icon)}${o.label}</button>`
          ).join("")}</div>
        </div>
        <div class="settings-row">
          <div><strong id="settings-transp-label">Reduzir transparência</strong>
            <p class="caption muted" id="settings-transp-desc">Troca os fundos translúcidos por cores sólidas. O GitPass já faz isso sozinho quando o Mac pede.</p></div>
          <label class="gp-switch"><input type="checkbox" role="switch" id="settings-transp" aria-labelledby="settings-transp-label" aria-describedby="settings-transp-desc" /><span class="gp-switch-track" aria-hidden="true"></span></label>
        </div>
      </section>

      <section class="surface" aria-labelledby="settings-h-integration">
        <h2 class="section-title" id="settings-h-integration">Integração com o Git</h2>
        <div class="settings-row">
          <div><strong>Identidade por projeto</strong><p class="caption muted" id="settings-int-desc">Cada repositório tem uma conta vinculada.</p></div>
          <span id="settings-int-badge">${loadingState}</span>
        </div>
        <div class="settings-row">
          <div><strong>Contas GitHub</strong><p class="caption muted">Perfis disponíveis para os seus projetos.</p></div>
          <span id="settings-acc-badge">${loadingState}</span>
        </div>
        <div class="settings-row">
          <div><strong>Integração, modo emergência e auditoria</strong><p class="caption muted">Ligue ou repare a integração e veja o que o GitPass bloqueou.</p></div>
          <div class="row-actions">
            <button type="button" class="control ghost" data-go="diagnostics">${icon("stethoscope")}Diagnóstico</button>
            <button type="button" class="control" data-go="integration">Abrir Integração${icon("chevronRight")}</button>
          </div>
        </div>
      </section>

      <section class="surface" aria-labelledby="settings-h-oauth">
        <h2 class="section-title" id="settings-h-oauth">GitHub</h2>
        <div class="settings-row">
          <div><strong>Conexão com o GitHub</strong><p class="caption muted" id="settings-oauth-desc">Verificando…</p></div>
          <span id="settings-oauth-badge">${loadingState}</span>
        </div>
      </section>

      <section class="surface" aria-labelledby="settings-h-data">
        <h2 class="section-title" id="settings-h-data">Dados e segurança</h2>
        <div class="settings-row settings-row-stack">
          <div><strong>Arquivo de configuração</strong></div>
          <div id="settings-config-path" class="settings-path-host"><span class="caption muted">Carregando…</span></div>
        </div>
        <div class="settings-row">
          <div><strong>Pasta de dados</strong><p class="caption muted">Abre a pasta onde o GitPass salva a configuração.</p></div>
          <button type="button" class="control" id="settings-open-data">${icon("folder")}<span>Abrir pasta de dados</span></button>
        </div>
        <div class="settings-row">
          <div><strong>Chaves do macOS (Keychain)</strong><p class="caption muted" id="settings-keychain-text">Carregando…</p></div>
          <span class="settings-row-icon" aria-hidden="true">${icon("key")}</span>
        </div>
      </section>

      <section class="surface" aria-labelledby="settings-h-about">
        <h2 class="section-title" id="settings-h-about">Sobre</h2>
        <div class="settings-row settings-about">
          ${brandSymbol()}
          <div><strong>GitPass 0.1.0</strong><p class="caption muted">A conta certa em cada projeto.</p></div>
        </div>
      </section>
    </div>
  `;

  const $ = <T extends HTMLElement>(id: string) => root.querySelector<T>(`#${id}`)!;

  // ----- Aparência -----
  const transp = $<HTMLInputElement>("settings-transp");
  const syncTheme = () => {
    const cur = storedTheme();
    for (const b of root.querySelectorAll<HTMLButtonElement>("[data-theme-mode]")) b.setAttribute("aria-pressed", String(b.dataset.themeMode === cur));
    transp.checked = reducedTransparency();
  };
  syncTheme();
  document.addEventListener(THEME_CHANGED_EVENT, syncTheme);
  transp.addEventListener("change", () => setReducedTransparency(transp.checked));

  root.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement;
    const modeBtn = target.closest<HTMLButtonElement>("[data-theme-mode]");
    if (modeBtn) return setTheme(modeBtn.dataset.themeMode as ThemeMode);
    const go = target.closest<HTMLElement>("[data-go]");
    if (go) ctx.navigate(go.dataset.go as "integration" | "diagnostics");
  });

  // ----- Integração (resumo; a tela própria tem as ações) -----
  const renderIntegration = (s: IntegrationStatus | null) => {
    if (!alive) return;
    if (!s) {
      $("settings-int-desc").textContent = "Não foi possível verificar. Abra Integração para ver o motivo.";
      $("settings-int-badge").innerHTML = `<span class="state error">${icon("xCircle")}Erro</span>`;
      return;
    }
    const h = integrationHeadline(s);
    $("settings-int-desc").textContent = h.kind === "success" ? "Cada repositório tem uma conta vinculada." : h.detail;
    const label = h.kind === "success" ? "Ativa" : s.enabled ? "Com problema" : "Desligada";
    $("settings-int-badge").innerHTML =
      h.kind === "success"
        ? `<span class="state">${icon("check")}${label}</span>`
        : `<span class="state ${h.kind}">${icon("alert")}${label}</span>`;
  };
  integrationApi
    .status()
    .then(renderIntegration)
    .catch(() => renderIntegration(null));
  let unlistenIntegration: (() => void) | null = null;
  void onIntegrationChanged(renderIntegration).then((un) => {
    if (alive) unlistenIntegration = un;
    else un();
  });

  const loadAccounts = (): void => {
    api
      .accounts()
      .then((list) => {
        if (!alive) return;
        const on = list.filter((a) => a.has_token).length;
        const off = list.length - on;
        $("settings-acc-badge").innerHTML =
          list.length === 0
            ? `<button type="button" class="control" data-go="accounts">${icon("plus")}Conectar conta</button>`
            : `<span class="state neutral">${icon("user")}${on === 1 ? "1 conectada" : `${on} conectadas`}</span>${
                off ? ` <span class="state warning">${icon("alert")}${off === 1 ? "1 desconectada" : `${off} desconectadas`}</span>` : ""
              }`;
      })
      .catch((e) => alive && ($("settings-acc-badge").innerHTML = `<span class="state error" title="${esc(errMessage(e))}">${icon("xCircle")}Erro</span>`));
  };
  loadAccounts();
  const onData = (): void => loadAccounts();
  document.addEventListener(DATA_CHANGED_EVENT, onData);

  // ----- GitHub -----
  const renderOauth = (st: OauthConfigStatus | null) => {
    const ready = st?.ok === true;
    $("settings-oauth-badge").innerHTML = ready ? `<span class="state">${icon("check")}Pronta</span>` : `<span class="state error">${icon("x")}Indisponível</span>`;
    $("settings-oauth-desc").textContent = ready ? "Pronta para conectar contas." : "Esta versão do app não tem o GitHub configurado. Atualize o GitPass.";
  };
  api
    .oauthStatus()
    .then((st) => alive && renderOauth(st))
    .catch(() => alive && renderOauth(null));

  // ----- Dados e segurança -----
  api
    .configPath()
    .then((p) => alive && ($("settings-config-path").innerHTML = pathBox(p, "o caminho")))
    .catch((e) => alive && ($("settings-config-path").innerHTML = errorView({ title: "Não foi possível ler o caminho", error: e, compact: true })));

  const openBtn = $<HTMLButtonElement>("settings-open-data");
  openBtn.addEventListener("click", () => {
    void withBusy(openBtn, "Abrindo…", async () => {
      try {
        await api.openDataFolder();
      } catch (e) {
        showToast(errMessage(e));
      }
    });
  });

  api
    .keychainHint()
    .then((t) => alive && ($("settings-keychain-text").textContent = t))
    .catch((e) => alive && ($("settings-keychain-text").textContent = errMessage(e)));

  return () => {
    alive = false;
    document.removeEventListener(THEME_CHANGED_EVENT, syncTheme);
    document.removeEventListener(DATA_CHANGED_EVENT, onData);
    unlistenIntegration?.();
  };
};
