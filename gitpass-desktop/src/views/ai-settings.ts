import "./ai-settings.css";
import { aiApi, OPENAI_BILLING_URL, type AiStatus } from "../lib/api-ai";
import { api } from "../lib/api";
import { errMessage, esc, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import type { MountView } from "../lib/view";
import { openModal } from "./repos-ui";

const AI_CONSENT_VERSION = 1;

function providerCard(
  id: string,
  name: string,
  desc: string,
  hasKey: boolean,
  keyValid: boolean,
  active: boolean,
  available: boolean,
  keyFormOpen: boolean
): string {
  const state = !available ? "soon" : keyValid ? "valid" : hasKey ? "ready" : "missing";
  const label = !available
    ? "Em breve"
    : keyValid
      ? "Chave válida"
      : hasKey
        ? "Chave salva"
        : "Sem chave";
  const showSaved = available && hasKey && !keyFormOpen;
  return `<div class="ai-provider-card" data-provider="${esc(id)}" data-active="${active}" data-key-mode="${showSaved ? "saved" : "edit"}">
    <div class="ai-provider-logo" data-provider="${esc(id)}">${esc(name[0])}</div>
    <div class="ai-provider-info">
      <strong>${esc(name)}</strong>
      <p>${esc(desc)}</p>
    </div>
    <span class="ai-provider-status" data-state="${state}">
      <span class="state-dot" data-on="${(hasKey || keyValid) && available}"></span>${esc(label)}
    </span>
    ${
      !available
        ? `<div class="ai-provider-actions"><span class="caption muted">Disponível na próxima versão.</span></div>`
        : showSaved
          ? `<div class="ai-provider-actions ai-key-saved">
              <div class="ai-key-saved-card">
                ${icon("lock", 18)}
                <div class="ai-key-saved-copy">
                  <strong>Chave cadastrada</strong>
                  <p>Armazenada com segurança no Keychain deste Mac. O valor não é exibido.</p>
                </div>
              </div>
              <div class="ai-key-saved-toolbar">
                <button type="button" class="control" data-test-key="${esc(id)}">${icon("activity", 14)}<span>Testar chave</span></button>
                <button type="button" class="control ghost" data-replace-key="${esc(id)}">${icon("edit", 14)}<span>Atualizar</span></button>
                <button type="button" class="control ghost danger" data-remove-key="${esc(id)}">${icon("trash", 14)}<span>Remover</span></button>
                <a class="control ghost" href="${esc(id === "openai" ? OPENAI_BILLING_URL : "#")}" rel="noopener noreferrer">${icon("external", 14)}<span>Billing</span></a>
              </div>
            </div>`
          : `<div class="ai-provider-actions">
              <form class="ai-key-group" data-key-form="${esc(id)}">
                <input type="password" autocomplete="off" spellcheck="false" placeholder="sk-…" aria-label="Chave ${esc(name)}" />
                <button type="submit" class="control primary">${hasKey ? "Salvar nova" : "Salvar"}</button>
                ${hasKey ? `<button type="button" class="control ghost" data-cancel-key-form="${esc(id)}">Cancelar</button>` : ""}
              </form>
              ${hasKey ? `<button type="button" class="control" data-test-key="${esc(id)}">${icon("activity", 14)}<span>Testar</span></button>` : ""}
              <a class="control ghost" href="${esc(id === "openai" ? OPENAI_BILLING_URL : "#")}" rel="noopener noreferrer">${icon("external", 14)}<span>Billing</span></a>
            </div>`
    }
  </div>`;
}

async function confirmEnableAi(): Promise<boolean> {
  const ok = await openModal<boolean>({
    title: "Ativar IA no GitPass",
    confirmLabel: "Ativar",
    body: `<p class="ai-modal-lead">Ao ativar, você confirma que:</p>
      <ul class="ai-modal-list">
        <li>Dados de repositório saem deste Mac <strong>apenas</strong> quando você usar um recurso de IA.</li>
        <li>O custo é na <strong>sua conta</strong> do provedor escolhido.</li>
        <li>E-mails <strong>nunca</strong> são enviados; autores são pseudonimizados por padrão.</li>
      </ul>`,
    read: () => true,
  });
  return ok === true;
}

async function confirmRemoveKey(name: string): Promise<boolean> {
  const ok = await openModal<boolean>({
    title: `Remover chave ${name}`,
    confirmLabel: "Remover",
    danger: true,
    body: `<p>A chave será apagada do Keychain. A IA será desligada se estiver usando este provedor.</p>`,
    read: () => true,
  });
  return ok === true;
}

export const mount: MountView = (root, ctx) => {
  let alive = true;
  let state: AiStatus | null = null;
  const keyValidProviders = new Set<string>();
  const keyFormOpen = new Set<string>();

  root.innerHTML = `
    <div class="page-head">
      <div class="heading-group">
        <h1>Inteligência artificial</h1>
        <p>Configure sua chave de IA e escolha o modelo.</p>
      </div>
      <button type="button" class="control" data-action="back">${icon("back")}<span>Configurações</span></button>
    </div>
    <div class="ai-page" id="ai-body">
      <div class="ai-hero">
        <div class="ai-hero-orb" aria-hidden="true">${icon("activity", 28)}</div>
        <div class="ai-hero-copy">
          <h2 class="ai-hero-title">IA com a sua chave</h2>
          <p class="ai-hero-sub">
            O GitPass monta o contexto dos seus repositórios e envia para o provedor que você escolher.
            Nenhum dado sai do Mac até você pedir, e a chave fica protegida no Keychain.
          </p>
          <span class="ai-hero-badge" id="ai-badge" data-tone="off">${icon("lock", 12)}Desligada</span>
        </div>
      </div>

      <section class="surface" aria-labelledby="ai-h-toggle">
        <h2 class="section-title" id="ai-h-toggle">Ativação</h2>
        <div class="ai-master-row">
          <div class="ai-master-text">
            <strong>Usar IA nos recursos do GitPass</strong>
            <p>Relatórios de commits, resumos e análises. Desligar não apaga a chave.</p>
          </div>
          <label class="gp-switch">
            <input type="checkbox" role="switch" id="ai-toggle" aria-label="Ativar IA" />
            <span class="gp-switch-track" aria-hidden="true"></span>
          </label>
        </div>
        <div class="ai-consent" id="ai-consent" hidden>
          <strong>${icon("alert", 14)} Aviso de privacidade</strong>
          <p>Ao ativar, você confirma que:</p>
          <ul>
            <li>Dados de repositório saem deste Mac <strong>apenas</strong> quando você usar um recurso de IA.</li>
            <li>O custo é na <strong>sua conta</strong> do provedor escolhido.</li>
            <li>E-mails <strong>nunca</strong> são enviados; autores são pseudonimizados por padrão.</li>
            <li>Se usar o nível gratuito do Gemini, os dados <strong>podem ser usados para treino</strong>.</li>
          </ul>
        </div>
        <div class="ai-activation-ready" id="ai-activation-ready" hidden>
          ${icon("shieldCheck", 20)}
          <div>
            <strong>Provedor pronto</strong>
            <p id="ai-activation-ready-copy">Chave salva. Ative o interruptor acima para usar IA nos recursos do GitPass.</p>
          </div>
        </div>
      </section>

      <section class="surface" aria-labelledby="ai-h-providers">
        <h2 class="section-title" id="ai-h-providers">Provedores</h2>
        <p class="ai-section-lead">Salve a chave do provedor que quiser usar. Hoje o GitPass trabalha com OpenAI; Anthropic e Gemini chegam em breve.</p>
        <div class="ai-providers" id="ai-providers"></div>
        <div id="ai-key-result" hidden></div>
      </section>

      <section class="surface" aria-labelledby="ai-h-model">
        <h2 class="section-title" id="ai-h-model">Modelo e idioma</h2>
        <dl>
          <div class="ai-model-row">
            <dt>Modelo OpenAI</dt>
            <dd><select id="ai-model" aria-label="Modelo"></select></dd>
          </div>
          <div class="ai-model-row">
            <dt>Idioma das respostas</dt>
            <dd><span class="mono" id="ai-locale">pt-BR</span></dd>
          </div>
          <div class="ai-model-row">
            <dt>Arquivo de configuração</dt>
            <dd><code class="mono" id="ai-config-path">…</code></dd>
          </div>
        </dl>
      </section>
    </div>
  `;

  const body = root.querySelector<HTMLElement>("#ai-body")!;
  const badge = root.querySelector<HTMLElement>("#ai-badge")!;
  const toggle = root.querySelector<HTMLInputElement>("#ai-toggle")!;
  const consentEl = root.querySelector<HTMLElement>("#ai-consent")!;
  const activationReady = root.querySelector<HTMLElement>("#ai-activation-ready")!;
  const providersEl = root.querySelector<HTMLElement>("#ai-providers")!;
  const resultEl = root.querySelector<HTMLElement>("#ai-key-result")!;
  const modelSel = root.querySelector<HTMLSelectElement>("#ai-model")!;
  const localeEl = root.querySelector<HTMLElement>("#ai-locale")!;
  const configEl = root.querySelector<HTMLElement>("#ai-config-path")!;

  function paint(s: AiStatus): void {
    state = s;

    badge.dataset.tone = s.enabled ? "on" : "off";
    badge.innerHTML = s.enabled
      ? `${icon("activity", 12)}Ligada — ${esc(s.provider || "openai")}`
      : `${icon("lock", 12)}Desligada`;

    toggle.checked = s.enabled;
    toggle.disabled = !s.openaiKeyConfigured;

    const ready = s.openaiKeyConfigured && !s.enabled;
    activationReady.hidden = !ready;
    consentEl.hidden = s.enabled || !s.openaiKeyConfigured;

    providersEl.innerHTML = [
      providerCard(
        "openai",
        "OpenAI",
        "GPT-4o, GPT-4o mini, o4-mini",
        s.openaiKeyConfigured,
        keyValidProviders.has("openai"),
        s.provider === "openai",
        true,
        keyFormOpen.has("openai")
      ),
      providerCard(
        "anthropic",
        "Anthropic",
        "Claude Sonnet, Opus, Haiku",
        s.anthropicKeyConfigured,
        keyValidProviders.has("anthropic"),
        s.provider === "anthropic",
        false,
        keyFormOpen.has("anthropic")
      ),
      providerCard(
        "google",
        "Google Gemini",
        "Gemini 2.5 Pro, Flash",
        s.googleKeyConfigured,
        keyValidProviders.has("google"),
        s.provider === "google",
        false,
        keyFormOpen.has("google")
      ),
    ].join("");

    modelSel.innerHTML = s.curatedModels
      .map((m) => `<option value="${esc(m)}"${m === s.model ? " selected" : ""}>${esc(m)}${m === s.recommendedModel ? " (recomendado)" : ""}</option>`)
      .join("");

    localeEl.textContent = s.responseLocale;
    configEl.textContent = s.configPath;
  }

  function showResult(msg: string, tone: "ok" | "err"): void {
    resultEl.hidden = false;
    resultEl.className = "ai-key-result";
    resultEl.dataset.tone = tone;
    resultEl.textContent = msg;
    window.setTimeout(() => {
      if (alive) resultEl.hidden = true;
    }, 8000);
  }

  aiApi
    .status()
    .then((s) => alive && paint(s))
    .catch((e) => alive && showToast(errMessage(e), "err"));

  toggle.addEventListener("change", () => {
    const want = toggle.checked;
    if (want && !state?.openaiKeyConfigured) {
      toggle.checked = false;
      showToast("Salve uma chave OpenAI antes de ativar a IA.", "err");
      return;
    }
    void (async () => {
      if (want) {
        const ok = await confirmEnableAi();
        if (!ok) {
          toggle.checked = false;
          return;
        }
      }
      try {
        const s = await aiApi.setEnabled(want, want ? AI_CONSENT_VERSION : state?.consentVersion ?? 0);
        paint(s);
        showToast(want ? "IA ativada." : "IA desligada.", "ok");
      } catch (e) {
        toggle.checked = !want;
        showToast(errMessage(e), "err");
      }
    })();
  });

  modelSel.addEventListener("change", () => {
    void aiApi
      .setPreferences({ model: modelSel.value })
      .then((s) => paint(s))
      .catch((e) => showToast(errMessage(e), "err"));
  });

  body.addEventListener("click", (ev) => {
    const t = ev.target as HTMLElement;

    if (t.closest('[data-action="back"]')) {
      ctx.navigate("settings");
      return;
    }

    const replaceBtn = t.closest<HTMLButtonElement>("[data-replace-key]");
    if (replaceBtn) {
      keyFormOpen.add(replaceBtn.dataset.replaceKey!);
      if (state) paint(state);
      return;
    }

    const cancelFormBtn = t.closest<HTMLButtonElement>("[data-cancel-key-form]");
    if (cancelFormBtn) {
      keyFormOpen.delete(cancelFormBtn.dataset.cancelKeyForm!);
      if (state) paint(state);
      return;
    }

    const testBtn = t.closest<HTMLButtonElement>("[data-test-key]");
    if (testBtn) {
      const pid = testBtn.dataset.testKey!;
      void withBusy(testBtn, "Testando…", () =>
        aiApi
          .testKey(pid)
          .then((msg) => {
            keyValidProviders.add(pid);
            if (state) paint(state);
            showResult(msg, "ok");
          })
          .catch((e) => {
            keyValidProviders.delete(pid);
            if (state) paint(state);
            showResult(errMessage(e), "err");
          })
      );
      return;
    }

    const removeBtn = t.closest<HTMLButtonElement>("[data-remove-key]");
    if (removeBtn) {
      const pid = removeBtn.dataset.removeKey!;
      const label = pid === "openai" ? "OpenAI" : pid === "anthropic" ? "Anthropic" : "Google Gemini";
      void (async () => {
        if (!(await confirmRemoveKey(label))) return;
        void withBusy(removeBtn, "…", async () => {
          try {
            const s = await aiApi.removeKey(pid);
            keyValidProviders.delete(pid);
            keyFormOpen.delete(pid);
            paint(s);
            showToast("Chave removida.", "ok");
          } catch (e) {
            showToast(errMessage(e), "err");
          }
        });
      })();
      return;
    }

    const link = t.closest<HTMLAnchorElement>("a[href]");
    if (link && /^https?:\/\//.test(link.href)) {
      ev.preventDefault();
      api.openExternal(link.href).catch((e) => showToast(errMessage(e), "err"));
    }
  });

  body.addEventListener("submit", (ev) => {
    ev.preventDefault();
    const form = (ev.target as HTMLElement).closest<HTMLFormElement>("[data-key-form]");
    if (!form) return;
    const pid = form.dataset.keyForm!;
    const input = form.querySelector<HTMLInputElement>("input")!;
    const key = input.value.trim();
    if (!key) return showToast("Informe a chave.", "err");
    const btn = form.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    void withBusy(btn, "Salvando…", async () => {
      try {
        const s = await aiApi.setKey(pid, key);
        input.value = "";
        keyFormOpen.delete(pid);
        keyValidProviders.delete(pid);
        paint(s);
        showResult("Chave salva no Keychain.", "ok");
      } catch (e) {
        showResult(errMessage(e), "err");
      }
    });
  });

  return () => {
    alive = false;
  };
};
