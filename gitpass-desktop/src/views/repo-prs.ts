import "./repo-prs.css";
import type { AccountRow } from "../lib/api";
import { bbPrApi, prApi, type RepoPullRequestRow } from "../lib/api-pr";
import type { RepoRow } from "../lib/api-repos";
import { errMessage, esc, relativeTime, showToast } from "../lib/dom";
import { errorView } from "../lib/errors";
import { icon } from "../lib/icons";
import { defaultBranchOf } from "./repo-branches";
import { openCreatePullRequest, openPullRequestPanel } from "./pr-panel";

type PrListState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ok"; items: RepoPullRequestRow[] }
  | { status: "error"; error: string }
  | { status: "unsupported"; reason: string };

export type RepoPrsDeps = {
  repo: () => RepoRow;
  account: () => AccountRow | undefined;
};

function isBitbucketRepo(repo: RepoRow): boolean {
  return (repo.remote_url ?? "").toLowerCase().includes("bitbucket.org");
}

function remoteCoords(repo: RepoRow): { owner: string; repo: string } | null {
  const owner = repo.github_owner?.trim() ?? "";
  const name = repo.github_repo?.trim() ?? "";
  if (!owner || !name) return null;
  return { owner, repo: name };
}

export function mountRepoPrsTab(el: HTMLElement, deps: RepoPrsDeps): { refresh: () => void; dispose: () => void } {
  let state: PrListState = { status: "idle" };
  let disposed = false;
  let seq = 0;

  const render = (): void => {
    const repo = deps.repo();
    const acc = deps.account();
    const coords = remoteCoords(repo);
    const provider = isBitbucketRepo(repo) ? "bitbucket" : "github";
    const hostLabel = provider === "bitbucket" ? "Bitbucket" : "GitHub";

    if (!coords) {
      el.innerHTML = `<section class="drawer-section repo-prs"><p class="caption muted">Este vínculo não aponta para um repositório remoto conhecido (sem workspace/repo no <span class="mono">origin</span>).</p></section>`;
      return;
    }
    if (!acc?.has_token) {
      el.innerHTML = `<section class="drawer-section repo-prs"><p class="caption muted">Conecte a conta vinculada para listar pull requests.</p></section>`;
      return;
    }

    const head = defaultBranchOf(repo.id)?.name ?? repo.branch ?? "main";
    const currentBranch = repo.branch ?? head;

    if (state.status === "unsupported") {
      el.innerHTML = `<section class="drawer-section repo-prs">
        <p class="caption muted">${esc(state.reason)}</p>
      </section>`;
      return;
    }

    if (state.status === "loading" || state.status === "idle") {
      el.innerHTML = `<section class="drawer-section repo-prs" aria-busy="true">
        <p class="caption muted"><span class="spinner" aria-hidden="true"></span> Buscando pull requests…</p>
      </section>`;
      return;
    }

    if (state.status === "error") {
      el.innerHTML = `<section class="drawer-section repo-prs">${errorView({
        title: "Não foi possível carregar os pull requests",
        error: state.error,
        compact: true,
        actions: [{ label: "Tentar de novo", action: "reload-prs", icon: "refresh" }],
      })}</section>`;
      return;
    }

    const canCreate =
      !!coords &&
      !!acc?.has_token &&
      !!currentBranch &&
      currentBranch !== head &&
      repo.exists &&
      !repo.account_missing;

    const toolbar = `<div class="repo-prs-toolbar">
      <button type="button" class="control" data-action="reload-prs">${icon("refresh")}<span>Atualizar</span></button>
      ${
        canCreate
          ? `<button type="button" class="control primary" data-action="create-pr">${icon("plus")}<span>Abrir PR deste branch</span></button>`
          : ""
      }
    </div>`;

    const list =
      state.items.length === 0
        ? `<p class="caption muted">Nenhum pull request aberto em <span class="mono">${esc(`${coords.owner}/${coords.repo}`)}</span> (${hostLabel}).</p>`
        : `<ul class="repo-prs-list">${state.items
            .map(
              (pr) =>
                `<li><button type="button" class="repo-prs-row" data-action="open-pr" data-number="${pr.number}">
                  ${icon("pullRequest", 16)}
                  <span class="repo-prs-row-text">
                    <strong>${esc(pr.title)}</strong>
                    <span class="caption muted">#${pr.number} · ${esc(pr.head_ref)} → ${esc(pr.base_ref)} · @${esc(pr.author_login)}${
                      pr.draft ? " · rascunho" : ""
                    } · ${esc(relativeTime(pr.updated_at))}</span>
                  </span>
                </button></li>`
            )
            .join("")}</ul>`;

    el.innerHTML = `<section class="drawer-section repo-prs">
      <div class="drawer-section-head"><h3>${icon("pullRequest")}Pull requests (${hostLabel})</h3></div>
      ${toolbar}
      ${list}
    </section>`;
  };

  const load = async (): Promise<void> => {
    const repo = deps.repo();
    const acc = deps.account();
    const coords = remoteCoords(repo);
    const provider = isBitbucketRepo(repo) ? "bitbucket" : "github";
    if (!coords || !acc?.has_token) {
      state = {
        status: "unsupported",
        reason: !coords
          ? "Este vínculo não aponta para um repositório remoto."
          : "Conecte a conta vinculada para listar pull requests.",
      };
      render();
      return;
    }
    const mySeq = ++seq;
    state = { status: "loading" };
    render();
    try {
      const items =
        provider === "bitbucket"
          ? await bbPrApi.listRepo(repo.account_id, coords.owner, coords.repo)
          : await prApi.listRepo(repo.account_id, coords.owner, coords.repo);
      if (disposed || mySeq !== seq) return;
      state = { status: "ok", items };
    } catch (e) {
      if (disposed || mySeq !== seq) return;
      state = { status: "error", error: errMessage(e) };
    }
    render();
  };

  el.addEventListener("click", (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("[data-action]");
    if (!btn || btn.disabled) return;
    const repo = deps.repo();
    const coords = remoteCoords(repo);
    const provider = isBitbucketRepo(repo) ? "bitbucket" : "github";
    if (!coords) return;
    const action = btn.dataset.action;
    if (action === "reload-prs") {
      void load();
      return;
    }
    if (action === "open-pr") {
      const num = Number(btn.dataset.number);
      if (!Number.isFinite(num)) return;
      openPullRequestPanel({
        provider,
        accountId: repo.account_id,
        owner: coords.owner,
        repo: coords.repo,
        number: num,
        onChanged: () => void load(),
      });
      return;
    }
    if (action === "create-pr") {
      const head = repo.branch ?? defaultBranchOf(repo.id)?.name ?? "";
      const base = defaultBranchOf(repo.id)?.name ?? "main";
      if (!head || head === base) {
        showToast("Envie um branch diferente do padrão antes de abrir um PR.", "err");
        return;
      }
      void (async () => {
        const created = await openCreatePullRequest(repo.account_id, coords.owner, coords.repo, head, base, provider);
        if (created) {
          openPullRequestPanel({
            provider,
            accountId: repo.account_id,
            owner: coords.owner,
            repo: coords.repo,
            number: created.number,
            onChanged: () => void load(),
          });
          void load();
        }
      })();
    }
  });

  void load();

  return {
    refresh: () => {
      void load();
    },
    dispose: () => {
      disposed = true;
    },
  };
}
