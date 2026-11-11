import "./pr-panel.css";
import { api } from "../lib/api";
import { bbPrApi, prApi, splitRepoFullName, type PrFileChange, type PullRequestDetail } from "../lib/api-pr";
import { errMessage, esc, showToast, withBusy } from "../lib/dom";
import { icon } from "../lib/icons";
import { openModal } from "./repos-ui";

const BB_PR_SCOPE_HINT = `Para aprovar, criar ou mesclar pull requests, o token precisa de <code>write:pullrequest:bitbucket</code> (API token Atlassian) ou <code>pullrequest:write</code> (OAuth). Sem isso o Bitbucket responde 403 — reconecte em Contas.`;

function bbPrWriteScopeToast(err: unknown): string {
  const msg = errMessage(err);
  if (
    /write:pullrequest|pullrequest:write|alterar pull requests|sem permissão para alterar/i.test(msg) ||
    (/403|recusou o token/i.test(msg) && /pull|aprovar|mesclar/i.test(msg))
  ) {
    return msg;
  }
  if (/403|recusou o token|permissão/i.test(msg)) {
    return `${msg} ${BB_PR_SCOPE_HINT.replace(/<[^>]+>/g, "")}`;
  }
  return msg;
}

function checkLabel(row: { status: string; conclusion: string | null }): string {
  if (row.status !== "completed") return "Em execução";
  const c = row.conclusion ?? "pending";
  switch (c) {
    case "success":
      return "Passou";
    case "failure":
      return "Falhou";
    case "neutral":
      return "Neutro";
    case "cancelled":
      return "Cancelado";
    case "skipped":
      return "Ignorado";
    case "timed_out":
      return "Tempo esgotado";
    case "action_required":
      return "Ação necessária";
    default:
      return c;
  }
}

function checkIcon(row: { status: string; conclusion: string | null }): string {
  if (row.status !== "completed") return icon("refresh", 12);
  const c = row.conclusion ?? "";
  if (c === "success" || c === "skipped" || c === "neutral") return icon("check", 12);
  if (c === "failure" || c === "timed_out" || c === "action_required") return icon("x", 12);
  return icon("alert", 12);
}

function parseHunkStart(header: string): number {
  const m = /\+(\d+)/.exec(header);
  return m ? Number(m[1]) : 0;
}

function parseHunkStartLeft(header: string): number {
  const m = /-(\d+)/.exec(header);
  return m ? Number(m[1]) : 0;
}

function patchToDiffHtml(
  patch: string,
  path: string,
  interactive: boolean,
  selected: { line: number; side: "LEFT" | "RIGHT" } | null
): string {
  const out: string[] = [];
  let rightLine = 0;
  let leftLine = 0;
  let inHunk = false;
  for (const raw of patch.split("\n")) {
    if (raw.startsWith("@@")) {
      inHunk = true;
      rightLine = parseHunkStart(raw);
      leftLine = parseHunkStartLeft(raw);
      out.push(`<div class="pr-diff-meta">${esc(raw)}</div>`);
      continue;
    }
    if (!inHunk) continue;
    if (raw.startsWith("+++") || raw.startsWith("---")) continue;
    if (raw.startsWith("-")) {
      const lineNum = leftLine;
      leftLine += 1;
      const sel = selected?.side === "LEFT" && selected.line === lineNum;
      if (interactive) {
        out.push(
          `<button type="button" class="pr-diff-line del${sel ? " sel" : ""}" data-pr-line-path="${esc(path)}" data-pr-line="${lineNum}" data-pr-line-side="LEFT" title="Comentar na linha removida ${lineNum}">
            <span class="pr-diff-gutter">${lineNum}</span><span class="pr-diff-text">${esc(raw)}</span></button>`
        );
      } else {
        out.push(`<div class="pr-diff-line del"><span class="pr-diff-gutter">${lineNum}</span><span class="pr-diff-text">${esc(raw)}</span></div>`);
      }
      continue;
    }
    if (!(raw.startsWith("+") || raw.startsWith(" "))) continue;
    const lineNum = rightLine;
    rightLine += 1;
    const cls = raw.startsWith("+") ? "add" : "ctx";
    const sel = selected?.side === "RIGHT" && selected.line === lineNum;
    if (interactive) {
      out.push(
        `<button type="button" class="pr-diff-line ${cls}${sel ? " sel" : ""}" data-pr-line-path="${esc(path)}" data-pr-line="${lineNum}" data-pr-line-side="RIGHT" title="Comentar na linha ${lineNum}">
          <span class="pr-diff-gutter">${lineNum}</span><span class="pr-diff-text">${esc(raw)}</span></button>`
      );
    } else {
      out.push(
        `<div class="pr-diff-line ${cls}"><span class="pr-diff-gutter">${lineNum}</span><span class="pr-diff-text">${esc(raw)}</span></div>`
      );
    }
  }
  return `<div class="pr-diff">${out.join("")}</div>`;
}

function reviewLabel(state: string): string {
  switch (state) {
    case "APPROVED":
      return "Aprovou";
    case "CHANGES_REQUESTED":
      return "Pediu alterações";
    case "COMMENTED":
      return "Comentou";
    case "DISMISSED":
      return "Revisão descartada";
    default:
      return state;
  }
}

type PrPanelOpts = {
  accountId: string;
  owner: string;
  repo: string;
  number: number;
  provider?: "github" | "bitbucket";
  onChanged?: () => void;
};

export function openPullRequestPanel(opts: PrPanelOpts): void {
  const backdrop = document.createElement("div");
  backdrop.className = "pr-backdrop modal-backdrop";
  backdrop.innerHTML = `<div class="pr-dialog" role="dialog" aria-modal="true" aria-labelledby="pr-panel-title">
    <header class="pr-dialog-head">
      <div><p class="caption muted">Carregando pull request…</p><h2 class="pr-dialog-title" id="pr-panel-title">…</h2></div>
      <button type="button" class="icon-button" data-pr-close aria-label="Fechar">${icon("x")}</button>
    </header>
    <div class="pr-dialog-body" data-pr-body><p class="caption muted"><span class="spinner"></span> Buscando no GitHub…</p></div>
    <footer class="pr-actions" data-pr-actions hidden></footer>
  </div>`;
  document.body.appendChild(backdrop);
  const dialog = backdrop.querySelector(".pr-dialog") as HTMLElement;
  const body = backdrop.querySelector("[data-pr-body]") as HTMLElement;
  const actions = backdrop.querySelector("[data-pr-actions]") as HTMLElement;
  const titleEl = backdrop.querySelector("#pr-panel-title") as HTMLElement;

  let disposed = false;
  let detail: PullRequestDetail | null = null;
  let files: PrFileChange[] | null = null;
  let filesError: string | null = null;
  let bbDiff: string | null = null;
  let bbDiffError: string | null = null;
  let reviewBody = "";
  let mergeMethod: "merge" | "squash" | "rebase" = "merge";
  const isBitbucket = opts.provider === "bitbucket";
  let pendingLineComment: { path: string; line: number; side: "LEFT" | "RIGHT" } | null = null;
  let lineCommentBody = "";
  let reviewerDraft = "";

  const filesSectionHtml = (d: PullRequestDetail): string => {
    if (files === null && !filesError) {
      return `<section class="pr-files"><h4>Alterações</h4><p class="caption muted"><span class="spinner" aria-hidden="true"></span> Carregando arquivos…</p></section>`;
    }
    if (filesError) {
      return `<section class="pr-files"><h4>Alterações</h4><p class="caption warn">${esc(filesError)}</p></section>`;
    }
    if (!files?.length) {
      return `<section class="pr-files"><h4>Alterações</h4><p class="caption muted">Nenhum arquivo listado.</p></section>`;
    }
    const shown = files.slice(0, 25);
    const more = files.length - shown.length;
    return `<section class="pr-files" aria-label="Arquivos alterados">
      <h4>Alterações <span class="caption muted">(${files.length})</span></h4>
      <div class="pr-files-list">${shown
        .map((f) => {
          const thread = d.comments
            .filter((c) => c.path === f.filename)
            .map(
              (c) =>
                `<div class="pr-inline-comment"><strong>@${esc(c.user_login)}</strong>${
                  c.line ? ` <span class="caption muted">linha ${c.line}</span>` : ""
                }<p>${esc(c.body)}</p></div>`
            )
            .join("");
          const canComment = d.can_review && d.state === "open" && !!f.patch && !!d.head_sha;
          const pick =
            pendingLineComment?.path === f.filename
              ? { line: pendingLineComment.line, side: pendingLineComment.side }
              : null;
          return `<details class="pr-file"${pendingLineComment?.path === f.filename ? " open" : ""}>
              <summary><code>${esc(f.filename)}</code><span class="caption muted">+${f.additions} / −${f.deletions}</span></summary>
              ${thread ? `<div class="pr-inline-thread">${thread}</div>` : ""}
              ${
                f.patch
                  ? canComment
                    ? patchToDiffHtml(f.patch, f.filename, true, pick)
                    : `<pre class="pr-patch">${esc(f.patch)}</pre>`
                  : `<p class="caption muted">Diff indisponível nesta prévia (muito grande ou binário). Use «Abrir no GitHub».</p>`
              }
              ${
                canComment && pendingLineComment?.path === f.filename
                  ? `<div class="pr-line-comment-form">
                      <p class="caption muted">Comentário na linha <strong>${pendingLineComment.line}</strong> (${pendingLineComment.side === "LEFT" ? "removida" : "nova"})</p>
                      <textarea class="input" rows="2" data-pr-line-body placeholder="Sugestão nesta linha…">${esc(lineCommentBody)}</textarea>
                      <div class="pr-line-comment-actions">
                        <button type="button" class="control sm primary" data-pr-line-send data-path="${esc(f.filename)}" data-line="${pendingLineComment.line}" data-side="${pendingLineComment.side}">${icon("send", 12)}<span>Enviar</span></button>
                        <button type="button" class="control sm" data-pr-line-cancel>Cancelar</button>
                      </div>
                    </div>`
                  : ""
              }
              ${
                canComment && !(pendingLineComment?.path === f.filename)
                  ? `<div class="pr-file-comment">
                      <p class="caption muted">Clique numa linha do diff ou envie um comentário geral no arquivo.</p>
                      <textarea class="input" rows="2" data-pr-file-input data-path="${esc(f.filename)}" placeholder="Comentário geral no arquivo…"></textarea>
                      <button type="button" class="control sm" data-pr-file-send data-path="${esc(f.filename)}">${icon("send", 12)}<span>Enviar</span></button>
                    </div>`
                  : ""
              }
            </details>`;
        })
        .join("")}</div>
      ${more > 0 ? `<p class="caption muted">Mais ${more} arquivo(s) — veja no GitHub.</p>` : ""}
    </section>`;
  };

  const close = (): void => {
    if (disposed) return;
    disposed = true;
    backdrop.remove();
  };

  const render = (): void => {
    if (!detail) return;
    titleEl.textContent = detail.title;
    const sub = backdrop.querySelector(".pr-dialog-head .caption.muted");
    if (sub) {
      sub.textContent = `#${detail.number} · ${detail.repo_full_name} · ${detail.head_ref} → ${detail.base_ref}`;
    }
    const failingChecks = detail.checks.filter(
      (c) => c.status === "completed" && (c.conclusion === "failure" || c.conclusion === "timed_out" || c.conclusion === "action_required")
    );
    const pendingChecks = detail.checks.filter((c) => c.status !== "completed");
    const mergeHint =
      detail.mergeable === false
        ? `<p class="caption warn">${icon("alert", 12)} Há conflitos ou checagens pendentes — o merge pode estar bloqueado.</p>`
        : failingChecks.length
          ? `<p class="caption warn">${icon("alert", 12)} ${failingChecks.length} checagem(ns) falhou(ram). O merge pode estar bloqueado no GitHub.</p>`
          : "";
    const checksBlock =
      detail.checks.length > 0
        ? `<section class="pr-checks" aria-label="Checagens CI">
            <h4>Checagens ${pendingChecks.length ? `<span class="caption muted">(${pendingChecks.length} em andamento)</span>` : ""}</h4>
            <ul class="pr-checks-list">${detail.checks
              .map(
                (c) =>
                  `<li class="pr-check-row">${checkIcon(c)}<span class="pr-check-name">${esc(c.name)}</span><span class="caption muted">${esc(checkLabel(c))}</span>${
                    c.html_url ? `<button type="button" class="control ghost sm" data-pr-check-url="${esc(c.html_url)}">${icon("external", 12)}</button>` : ""
                  }</li>`
              )
              .join("")}</ul>
          </section>`
        : "";
    const bbScopeCallout =
      isBitbucket && detail.state === "open" && (detail.can_review || detail.can_merge)
        ? `<p class="caption warn bb-pr-scope-hint">${BB_PR_SCOPE_HINT}</p>`
        : "";
    body.innerHTML = `
      ${isBitbucket ? `<p class="caption muted">Bitbucket: diff abaixo; comentários linha a linha no site.</p>` : ""}
      ${bbScopeCallout}
      ${mergeHint}
      ${isBitbucket ? "" : checksBlock}
      <div class="pr-stats">
        <span>+${detail.additions.toLocaleString("pt-BR")} / −${detail.deletions.toLocaleString("pt-BR")}</span>
        <span>${detail.changed_files} arquivo(s)</span>
        <span>Por @${esc(detail.author_login)}</span>
        <span>${detail.draft ? "Rascunho" : detail.state}</span>
      </div>
      ${detail.body ? `<p class="pr-body-text">${esc(detail.body)}</p>` : ""}
      ${
        isBitbucket
          ? bbDiffError
            ? `<p class="caption warn">${esc(bbDiffError)}</p>`
            : bbDiff
              ? `<section class="pr-files"><h4>Diff</h4><pre class="pr-patch">${esc(bbDiff)}</pre></section>`
              : `<p class="caption muted"><span class="spinner"></span> Carregando diff…</p>`
          : filesSectionHtml(detail)
      }
      ${
        detail.can_request_reviewers
          ? `<section class="pr-reviewers" aria-label="Revisores">
              <h4>Revisores</h4>
              ${
                detail.requested_reviewers.length
                  ? `<p class="caption muted">Pedidos: ${detail.requested_reviewers.map((r) => `@${esc(r)}`).join(", ")}</p>`
                  : `<p class="caption muted">Ninguém convidado ainda.</p>`
              }
              <div class="pr-reviewers-form">
                <input class="input" type="text" data-pr-reviewer-input placeholder="login GitHub (ex.: octocat)" value="${esc(reviewerDraft)}" />
                <button type="button" class="control sm" data-pr-reviewer-add>${icon("user", 12)}<span>Pedir revisão</span></button>
              </div>
            </section>`
          : detail.requested_reviewers.length
            ? `<section class="pr-reviewers"><h4>Revisores</h4><p class="caption muted">${detail.requested_reviewers.map((r) => `@${esc(r)}`).join(", ")}</p></section>`
            : ""
      }
      <section class="pr-reviews" aria-label="Revisões">
        <h4>Revisões</h4>
        ${
          detail.reviews.length
            ? detail.reviews
                .map(
                  (r) =>
                    `<div class="pr-review-row"><strong>@${esc(r.user_login)}</strong> · ${esc(reviewLabel(r.state))}${
                      r.body ? `<span>${esc(r.body)}</span>` : ""
                    }</div>`
                )
                .join("")
            : `<p class="caption muted">Nenhuma revisão ainda.</p>`
        }
      </section>
      ${
        detail.can_review && detail.state === "open"
          ? `<div class="pr-field"><label><span>Comentário (obrigatório para pedir alterações)</span>
            <textarea class="input" data-pr-review-body placeholder="Feedback para o autor…">${esc(reviewBody)}</textarea></label></div>`
          : ""
      }`;
    const ta = body.querySelector<HTMLTextAreaElement>("[data-pr-review-body]");
    ta?.addEventListener("input", () => {
      reviewBody = ta.value;
    });
    const reviewerInput = body.querySelector<HTMLInputElement>("[data-pr-reviewer-input]");
    reviewerInput?.addEventListener("input", () => {
      reviewerDraft = reviewerInput.value;
    });
    const lineBody = body.querySelector<HTMLTextAreaElement>("[data-pr-line-body]");
    lineBody?.addEventListener("input", () => {
      lineCommentBody = lineBody.value;
    });

    actions.hidden = false;
    actions.innerHTML = `
      <button type="button" class="control" data-pr-external>${icon("external")}<span>Abrir no ${isBitbucket ? "Bitbucket" : "GitHub"}</span></button>
      ${
        detail.can_review && detail.state === "open"
          ? isBitbucket
            ? `<button type="button" class="control primary" data-pr-approve>${icon("check")}<span>Aprovar</span></button>`
            : `<button type="button" class="control primary" data-pr-approve>${icon("check")}<span>Aprovar</span></button>
             <button type="button" class="control" data-pr-changes>${icon("alert")}<span>Pedir alterações</span></button>`
          : ""
      }
      ${
        detail.can_merge && detail.state === "open"
          ? `${
              !isBitbucket && detail.can_enable_auto_merge
                ? detail.auto_merge_enabled
                  ? `<button type="button" class="control" data-pr-auto-off>${icon("alert")}<span>Desativar auto-merge</span></button>`
                  : `<button type="button" class="control" data-pr-auto-on>${icon("check")}<span>Auto-merge quando passar</span></button>`
                : ""
            }${
              isBitbucket
                ? ""
                : `<label class="pr-merge-pick"><span class="caption muted">Como mesclar</span>
              <select class="input" data-pr-merge-method>
                <option value="merge"${mergeMethod === "merge" ? " selected" : ""}>Merge commit</option>
                <option value="squash"${mergeMethod === "squash" ? " selected" : ""}>Squash</option>
                <option value="rebase"${mergeMethod === "rebase" ? " selected" : ""}>Rebase</option>
              </select></label>`
            }
             <button type="button" class="control primary" data-pr-merge>${icon("merge")}<span>Mesclar</span></button>`
          : ""
      }
      <button type="button" class="control" data-pr-close-inner>Fechar</button>`;
  };

  const reload = async (): Promise<void> => {
    try {
      detail = isBitbucket
        ? await bbPrApi.detail(opts.accountId, opts.owner, opts.repo, opts.number)
        : await prApi.detail(opts.accountId, opts.owner, opts.repo, opts.number);
      files = null;
      filesError = null;
      dialog.classList.toggle("pr-dialog-wide", true);
      bbDiff = null;
      bbDiffError = null;
      render();
      if (isBitbucket) {
        try {
          bbDiff = await bbPrApi.diff(opts.accountId, opts.owner, opts.repo, opts.number);
        } catch (e) {
          bbDiffError = errMessage(e);
        }
        if (!disposed) render();
      } else {
        try {
          files = await prApi.files(opts.accountId, opts.owner, opts.repo, opts.number);
        } catch (e) {
          filesError = errMessage(e);
        }
        if (!disposed) render();
      }
    } catch (e) {
      body.innerHTML = `<p class="caption err">${esc(errMessage(e))}</p>`;
      actions.hidden = true;
    }
  };

  backdrop.addEventListener("change", (ev) => {
    const sel = (ev.target as HTMLElement).closest<HTMLSelectElement>("[data-pr-merge-method]");
    if (!sel) return;
    const v = sel.value;
    if (v === "merge" || v === "squash" || v === "rebase") mergeMethod = v;
  });

  backdrop.addEventListener("click", (ev) => {
    const t = ev.target as HTMLElement;
    if (t === backdrop || t.closest("[data-pr-close]") || t.closest("[data-pr-close-inner]")) {
      ev.preventDefault();
      close();
    }
    if (t.closest("[data-pr-external]") && detail) {
      void api.openExternal(detail.html_url);
    }
    const checkUrl = t.closest<HTMLElement>("[data-pr-check-url]");
    if (checkUrl?.dataset.prCheckUrl) {
      void api.openExternal(checkUrl.dataset.prCheckUrl);
    }
    const fileSend = t.closest<HTMLButtonElement>("[data-pr-file-send]");
    const linePick = t.closest<HTMLButtonElement>("[data-pr-line]");
    if (linePick?.dataset.prLinePath && linePick.dataset.prLine) {
      const side = linePick.dataset.prLineSide === "LEFT" ? "LEFT" : "RIGHT";
      pendingLineComment = { path: linePick.dataset.prLinePath, line: Number(linePick.dataset.prLine), side };
      render();
      body.querySelector<HTMLTextAreaElement>("[data-pr-line-body]")?.focus();
      return;
    }
    if (t.closest("[data-pr-line-cancel]")) {
      pendingLineComment = null;
      lineCommentBody = "";
      render();
      return;
    }
    const lineSend = t.closest<HTMLButtonElement>("[data-pr-line-send]");
    const headForComment = detail?.head_sha;
    if (lineSend?.dataset.path && lineSend.dataset.line && headForComment) {
      const path = lineSend.dataset.path;
      const line = Number(lineSend.dataset.line);
      const side = lineSend.dataset.side === "LEFT" ? "LEFT" : "RIGHT";
      const text = lineCommentBody.trim();
      if (!text) {
        showToast("Escreva o comentário.", "err");
        return;
      }
      void withBusy(lineSend, "Enviando…", async () => {
        await prApi.commentFile(opts.accountId, opts.owner, opts.repo, opts.number, headForComment, path, text, line, side);
        showToast("Comentário enviado na linha.", "ok");
        pendingLineComment = null;
        lineCommentBody = "";
        opts.onChanged?.();
        await reload();
      }).catch((e) => showToast(errMessage(e), "err"));
      return;
    }
    if (fileSend?.dataset.path && headForComment) {
      const path = fileSend.dataset.path;
      const headSha = headForComment;
      const wrap = fileSend.closest(".pr-file");
      const input = wrap?.querySelector<HTMLTextAreaElement>(`[data-pr-file-input][data-path="${CSS.escape(path)}"]`);
      const text = input?.value.trim() ?? "";
      if (!text) {
        showToast("Escreva o comentário.", "err");
        return;
      }
      void withBusy(fileSend, "Enviando…", async () => {
        await prApi.commentFile(opts.accountId, opts.owner, opts.repo, opts.number, headSha, path, text);
        showToast("Comentário enviado no arquivo.", "ok");
        if (input) input.value = "";
        opts.onChanged?.();
        await reload();
      }).catch((e) => showToast(errMessage(e), "err"));
      return;
    }
    if (t.closest("[data-pr-auto-on]") && detail) {
      void withBusy(t.closest("button")!, "Ativando…", async () => {
        await prApi.enableAutoMerge(opts.accountId, opts.owner, opts.repo, opts.number, mergeMethod);
        showToast("Auto-merge ativado.", "ok");
        await reload();
      }).catch((e) => showToast(errMessage(e), "err"));
      return;
    }
    if (t.closest("[data-pr-auto-off]") && detail) {
      void withBusy(t.closest("button")!, "Desativando…", async () => {
        await prApi.disableAutoMerge(opts.accountId, opts.owner, opts.repo, opts.number);
        showToast("Auto-merge desativado.", "ok");
        await reload();
      }).catch((e) => showToast(errMessage(e), "err"));
      return;
    }
    const reviewerAdd = t.closest<HTMLButtonElement>("[data-pr-reviewer-add]");
    if (reviewerAdd && detail?.can_request_reviewers) {
      const login = reviewerDraft.trim();
      if (!login) {
        showToast("Informe o login do revisor.", "err");
        return;
      }
      void withBusy(reviewerAdd, "Convidando…", async () => {
        await prApi.requestReviewers(opts.accountId, opts.owner, opts.repo, opts.number, [login]);
        showToast("Pedido de revisão enviado.", "ok");
        reviewerDraft = "";
        opts.onChanged?.();
        await reload();
      }).catch((e) => showToast(errMessage(e), "err"));
      return;
    }
    const btn = t.closest<HTMLButtonElement>("[data-pr-approve],[data-pr-changes],[data-pr-merge]");
    if (!btn || !detail) return;
    void (async () => {
      try {
        if (btn.hasAttribute("data-pr-approve")) {
          if (isBitbucket) {
            await withBusy(btn, "Enviando…", () =>
              bbPrApi.approve(opts.accountId, opts.owner, opts.repo, opts.number)
            );
          } else {
            await withBusy(btn, "Enviando…", () =>
              prApi.review(opts.accountId, opts.owner, opts.repo, opts.number, "approve", reviewBody.trim() || null)
            );
          }
          showToast("Revisão: aprovado.", "ok");
        } else if (btn.hasAttribute("data-pr-changes")) {
          const text = reviewBody.trim();
          if (!text) {
            showToast("Escreva o que precisa mudar.", "err");
            return;
          }
          await withBusy(btn, "Enviando…", () =>
            prApi.review(opts.accountId, opts.owner, opts.repo, opts.number, "request_changes", text)
          );
          showToast("Pedido de alterações enviado.", "ok");
        } else if (btn.hasAttribute("data-pr-merge")) {
          await withBusy(btn, "Mesclando…", () =>
            isBitbucket
              ? bbPrApi.merge(opts.accountId, opts.owner, opts.repo, opts.number)
              : prApi.merge(opts.accountId, opts.owner, opts.repo, opts.number, mergeMethod)
          );
          showToast("Pull request mesclado.", "ok");
        }
        opts.onChanged?.();
        await reload();
      } catch (e) {
        showToast(isBitbucket ? bbPrWriteScopeToast(e) : errMessage(e), "err");
      }
    })();
  });

  document.addEventListener(
    "keydown",
    (ev) => {
      if (ev.key === "Escape" && !disposed) {
        ev.preventDefault();
        close();
      }
    },
    { once: false }
  );

  void reload();
}

export async function openCreatePullRequest(
  accountId: string,
  owner: string,
  repo: string,
  head: string,
  baseDefault: string,
  provider: "github" | "bitbucket" = "github"
): Promise<{ number: number } | null> {
  const titleDefault = `Atualizar ${head}`;
  const picked = await openModal<{ title: string; base: string; body: string; draft: boolean }>({
    title: "Abrir pull request",
    wide: true,
    confirmLabel: "Criar PR",
    body: `<p class="caption muted">Envia <code>${esc(head)}</code> para o branch base no GitHub.</p>
      <label class="pr-field"><span class="caption muted">Título</span>
        <input class="input" type="text" data-pr-title value="${esc(titleDefault)}" /></label>
      <label class="pr-field"><span class="caption muted">Branch base</span>
        <input class="input" type="text" data-pr-base value="${esc(baseDefault)}" autocomplete="off" /></label>
      <label class="pr-field"><span class="caption muted">Descrição (opcional)</span>
        <textarea class="input" data-pr-desc rows="4" placeholder="O que mudou e por quê"></textarea></label>
      <label class="repos-check"><input type="checkbox" data-pr-draft /><span>Criar como rascunho</span></label>`,
    read: (dialog) => {
      const title = dialog.querySelector<HTMLInputElement>("[data-pr-title]")?.value.trim() ?? "";
      const base = dialog.querySelector<HTMLInputElement>("[data-pr-base]")?.value.trim() ?? "";
      const body = dialog.querySelector<HTMLTextAreaElement>("[data-pr-desc]")?.value.trim() ?? "";
      const draft = dialog.querySelector<HTMLInputElement>("[data-pr-draft]")?.checked ?? false;
      if (!title || !base) return undefined;
      return { title, base, body, draft };
    },
  });
  if (!picked) return null;
  try {
    const created =
      provider === "bitbucket"
        ? await bbPrApi.create(accountId, owner, repo, picked.title, head, picked.base, picked.body || null, picked.draft)
        : await prApi.create(accountId, owner, repo, picked.title, head, picked.base, picked.body || null, picked.draft);
    showToast(`PR #${created.number} criado.`, "ok");
    return { number: created.number };
  } catch (e) {
    showToast(provider === "bitbucket" ? bbPrWriteScopeToast(e) : errMessage(e), "err");
    return null;
  }
}

export async function openCreatePullRequestWizard(accountId: string, onCreated?: () => void): Promise<void> {
  const picked = await openModal<{
    owner: string;
    repo: string;
    head: string;
    base: string;
    title: string;
    body: string;
    draft: boolean;
  }>({
    title: "Novo pull request",
    wide: true,
    confirmLabel: "Criar",
    body: `<p class="caption muted">Repositório no GitHub e branches já enviados (push).</p>
      <div class="pr-field"><label><span class="caption muted">Owner</span><input class="input" data-pr-owner placeholder="sua-org" /></label></div>
      <div class="pr-field"><label><span class="caption muted">Repositório</span><input class="input" data-pr-repo placeholder="meu-repo" /></label></div>
      <div class="pr-field"><label><span class="caption muted">Branch de origem (head)</span><input class="input" data-pr-head placeholder="feature/minha-mudanca" /></label></div>
      <div class="pr-field"><label><span class="caption muted">Branch base</span><input class="input" data-pr-base value="main" /></label></div>
      <div class="pr-field"><label><span class="caption muted">Título</span><input class="input" data-pr-title placeholder="Resumo da mudança" /></label></div>
      <div class="pr-field"><label><span class="caption muted">Descrição</span><textarea class="input" data-pr-desc rows="3"></textarea></label></div>
      <label class="repos-check"><input type="checkbox" data-pr-draft /><span>Rascunho</span></label>`,
    read: (dialog) => {
      const owner = dialog.querySelector<HTMLInputElement>("[data-pr-owner]")?.value.trim() ?? "";
      const repo = dialog.querySelector<HTMLInputElement>("[data-pr-repo]")?.value.trim() ?? "";
      const head = dialog.querySelector<HTMLInputElement>("[data-pr-head]")?.value.trim() ?? "";
      const base = dialog.querySelector<HTMLInputElement>("[data-pr-base]")?.value.trim() ?? "main";
      const title = dialog.querySelector<HTMLInputElement>("[data-pr-title]")?.value.trim() ?? "";
      const body = dialog.querySelector<HTMLTextAreaElement>("[data-pr-desc]")?.value.trim() ?? "";
      const draft = dialog.querySelector<HTMLInputElement>("[data-pr-draft]")?.checked ?? false;
      if (!owner || !repo || !head || !base || !title) return undefined;
      return { owner, repo, head, base, title, body, draft };
    },
  });
  if (!picked) return;
  const created = await prApi.create(
    accountId,
    picked.owner,
    picked.repo,
    picked.title,
    picked.head,
    picked.base,
    picked.body || null,
    picked.draft
  );
  showToast(`PR #${created.number} criado.`, "ok");
  onCreated?.();
  const parts = { owner: picked.owner, repo: picked.repo };
  openPullRequestPanel({ accountId, ...parts, number: created.number, onChanged: onCreated });
}

export function openPullRequestFromInbox(
  accountId: string,
  pr: { repo_full_name: string; number: number },
  onChanged?: () => void,
  provider: "github" | "bitbucket" = "github"
): void {
  const parts = splitRepoFullName(pr.repo_full_name);
  if (!parts) {
    showToast("Repositório do PR inválido.", "err");
    return;
  }
  openPullRequestPanel({
    provider,
    accountId,
    owner: parts.owner,
    repo: parts.repo,
    number: pr.number,
    onChanged,
  });
}
