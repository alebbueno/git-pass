import { api, type AccountRow } from "./lib/api";
import { reposApi, type RepoRow } from "./lib/api-repos";
import { errMessage, esc, showToast } from "./lib/dom";
import { icon } from "./lib/icons";
import { profileName } from "./lib/profile";
import { drawerRoute } from "./views/repo-detail";
import { projectName } from "./views/repos-ui";
import type { ViewContext, ViewId } from "./lib/view";

type PaletteItem = {
  id: string;
  label: string;
  hint?: string;
  group: string;
  run: () => void | Promise<void>;
};

let openSeq = 0;
let opener: HTMLElement | null = null;

export function installCommandPalette(ctx: {
  navigate: (route: string) => void;
  viewRoot: HTMLElement;
  getView: () => ViewId;
}): void {
  document.addEventListener("keydown", (e) => {
    if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "k" || e.altKey || e.shiftKey) return;
    if (document.querySelector(".command-palette-backdrop, .modal-backdrop")) return;
    e.preventDefault();
    void openPalette(ctx);
  });
}

async function openPalette(ctx: { navigate: (route: string) => void }): Promise<void> {
  const seq = ++openSeq;
  opener = document.activeElement as HTMLElement | null;
  let repos: RepoRow[] = [];
  let accounts: AccountRow[] = [];
  try {
    [accounts, repos] = await Promise.all([api.accounts(), reposApi.list()]);
  } catch (e) {
    showToast(errMessage(e), "err");
    return;
  }
  if (seq !== openSeq) return;

  const backdrop = document.createElement("div");
  backdrop.className = "command-palette-backdrop";
  backdrop.innerHTML = `<div class="command-palette" role="dialog" aria-modal="true" aria-label="Paleta de comandos">
    <label class="command-palette-search">${icon("search")}<input type="search" placeholder="Projeto, ação ou perfil…" spellcheck="false" autocomplete="off" /></label>
    <ul class="command-palette-list" role="listbox"></ul>
    <p class="command-palette-hint muted small">↑↓ seleciona · Enter executa · Esc fecha</p>
  </div>`;
  document.body.appendChild(backdrop);
  const input = backdrop.querySelector<HTMLInputElement>("input")!;
  const list = backdrop.querySelector<HTMLUListElement>(".command-palette-list")!;
  let sel = 0;

  const itemsFor = (q: string): PaletteItem[] => {
    const t = q.trim().toLowerCase();
    const out: PaletteItem[] = [];
    const match = (s: string) => !t || s.toLowerCase().includes(t);
    for (const r of repos.filter((b) => b.exists).slice(0, 12)) {
      const name = projectName(r);
      if (!match(name) && !match(r.root_path)) continue;
      out.push({
        id: `go-${r.id}`,
        label: name,
        hint: r.root_path,
        group: "Ir para repositório",
        run: () => ctx.navigate(drawerRoute(r.id)),
      });
    }
    const pick = repos.find((r) => r.exists);
    if (pick) {
      const base = [
        {
          id: "identity",
          label: `Revisar identidade — ${projectName(pick)}`,
          group: "Ações",
          run: () => ctx.navigate(drawerRoute(pick.id, "overview")),
        },
        {
          id: "finder",
          label: `Mostrar no Finder — ${projectName(pick)}`,
          group: "Ações",
          run: () => reposApi.openInFinder(pick.root_path),
        },
        {
          id: "terminal",
          label: `Abrir no Terminal — ${projectName(pick)}`,
          group: "Ações",
          run: () => reposApi.openInTerminal(pick.root_path),
        },
        {
          id: "cursor",
          label: `Abrir no Cursor — ${projectName(pick)}`,
          group: "Ações",
          run: () => reposApi.openInEditor(pick.root_path, "Cursor"),
        },
        {
          id: "code",
          label: `Abrir no VS Code — ${projectName(pick)}`,
          group: "Ações",
          run: () => reposApi.openInEditor(pick.root_path, "Visual Studio Code"),
        },
      ];
      for (const it of base) {
        if (match(it.label)) out.push(it);
      }
    }
    for (const a of accounts) {
      const label = profileName(a);
      if (match(label) || match(a.login ?? "")) {
        out.push({
          id: `acc-${a.id}`,
          label,
          hint: a.login ? `@${a.login}` : undefined,
          group: "Filtrar por perfil",
          run: () => ctx.navigate(`repos?profile=${encodeURIComponent(a.id)}`),
        });
      }
    }
    return out.slice(0, 24);
  };

  const render = (): void => {
    const items = itemsFor(input.value);
    sel = Math.min(sel, Math.max(0, items.length - 1));
    list.innerHTML = items
      .map(
        (it, i) =>
          `<li role="option" aria-selected="${i === sel}"><button type="button" data-idx="${i}" class="${i === sel ? "active" : ""}">
            <span class="command-palette-group">${esc(it.group)}</span>
            <span class="command-palette-label">${esc(it.label)}</span>
            ${it.hint ? `<span class="command-palette-sub muted">${esc(it.hint)}</span>` : ""}
          </button></li>`
      )
      .join("");
    list.querySelector<HTMLButtonElement>(`[data-idx="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  };

  const close = (): void => {
    backdrop.remove();
    if (opener?.isConnected) opener.focus();
    opener = null;
  };

  const run = async (idx: number): Promise<void> => {
    const items = itemsFor(input.value);
    const it = items[idx];
    if (!it) return;
    close();
    try {
      await it.run();
    } catch (e) {
      showToast(errMessage(e), "err");
    }
  };

  input.addEventListener("input", () => {
    sel = 0;
    render();
  });
  input.addEventListener("keydown", (e) => {
    const items = itemsFor(input.value);
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      sel = (sel + 1) % Math.max(1, items.length);
      render();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      sel = (sel - 1 + items.length) % Math.max(1, items.length);
      render();
    } else if (e.key === "Enter") {
      e.preventDefault();
      void run(sel);
    }
  });
  backdrop.addEventListener("pointerdown", (e) => {
    if (e.target === backdrop) close();
  });
  list.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-idx]");
    if (!btn) return;
    void run(Number(btn.dataset.idx));
  });
  render();
  input.focus();
}
