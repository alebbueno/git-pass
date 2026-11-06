# GitPass

Desktop app (macOS first) to manage **multiple GitHub accounts** per machine, with a background agent and validation hooks. See [`docs/gitpass-v1-design.md`](docs/gitpass-v1-design.md) for the product spec.

## Repo layout

| Path | Role |
|------|------|
| `crates/gitpass-core` | Shared types, paths, IPC message shapes |
| `crates/gitpassd` | Background daemon (Unix socket server) |
| `crates/gitpass-cli` | `gitpass` + `git-credential-gitpass` (HTTPS helper) |
| `gitpass-desktop` | Tauri + Vite UI shell (menu bar / windows) |

## Prerequisites

- **Rust** (`rustup`, stable) — install from <https://rustup.rs/>
- **Node.js** 20+ (for the Tauri frontend)
- **macOS 14+** (target platform for v1)

## Build (daemon + CLI)

```bash
cargo build -p gitpassd -p gitpass-cli --release
```

Run the daemon in one terminal:

```bash
cargo run -p gitpassd --bin gitpassd
```

In another:

```bash
cargo run -p gitpass-cli --bin gitpass -- ping
# ok — gitpassd 0.1.0
```

Socket path: `~/Library/Application Support/com.gitpass.GitPass/gitpassd.sock`

### Git hooks (global)

Install scripts under **`~/Library/Application Support/com.gitpass.GitPass/hooks`** and point Git at them:

```bash
cargo run -p gitpass-cli --bin gitpass -- hooks install
gitpass doctor
```

- Sets **`git config --global core.hooksPath`** to that directory (hooks call `gitpass hook run --phase <name>`).
- **Validation:** `gitpass hook` sends **`ValidateHook`** to **`gitpassd`**. The daemon reloads **`config.toml`**, checks binding / `github_login` / `origin`, and if **account login ≠ `origin` owner** (e.g. org repo or fork workflow) it uses the **OAuth token in Keychain** to call **`GET /repos/{owner}/{repo}`** and confirm **push** permission. If the API is unreachable in that ambiguous case, validation **allows with a warning** (design: “not confirmed”) unless **`GITPASS_STRICT_API=1`** is set in **gitpassd’s** environment — then it **blocks** with `GP-API`. **Offline** hook fallback (no daemon) does **not** use the API (use daemon for collaborator checks).  
- **Strict API:** run `gitpassd` with `GITPASS_STRICT_API=1` to block when GitHub API fails instead of allowing unconfirmed.
- **Bypass CI:** `CI` + **`GITPASS_ALLOW_CI=1`** is evaluated in the **hook process** (not in the daemon) and skips checks.
- **`post-commit`** does not enforce rules (too late to block a commit).
- If you already had **`core.hooksPath`**, the previous value is saved as **`core.hooksPath.gitpass-backup`** before overwriting.
- To restore the old hook path:  
  `git config --global core.hooksPath "$(git config --global --get core.hooksPath.gitpass-backup)"`

### `gitpass doctor`

Checks **macOS version**, **`git` on PATH**, **Application Support**, **`config.toml`**, **hooks** (present/up to date), **`core.hooksPath`**, **`gitpassd` socket**, and prints the **`gitpass` binary path** used in hook scripts.

Exit status: **0** = all ok, **1** = errors, **2** = warnings only.

```bash
cargo run -p gitpass-cli --bin gitpass -- doctor
```

### Configuration (non-secrets)

On first start, `gitpassd` creates **`~/Library/Application Support/com.gitpass.GitPass/config.toml`** if it is missing. This file holds **only non-sensitive** data (account ids, repo bindings, UI flags). **OAuth access tokens** are stored in the **macOS Keychain** (service `com.gitpass.GitPass`, account `github.oauth.<uuid>`).

### Accounts and repo bindings (CLI)

From any directory:

```bash
# Add a GitHub account (login + auth mode)
cargo run -p gitpass-cli --bin gitpass -- account add --login octocat --label "Personal" --auth ssh
cargo run -p gitpass-cli --bin gitpass -- account list

# Easiest HTTPS: register Git credential helper (token from Keychain; picks account by **local** repo binding first, then by owner/repo in the URL)
cargo build -p gitpass-cli --bins
cargo run -p gitpass-cli --bin gitpass -- config install-credential-helper

# Sign in with GitHub (browser) — creates account + saves token in Keychain
export GITHUB_OAUTH_CLIENT_ID="Ov23..."        # from https://github.com/settings/developers
export GITHUB_OAUTH_CLIENT_SECRET="..."
cargo run -p gitpass-cli --bin gitpass -- account login --new

# Refresh token for an existing account (id or login)
cargo run -p gitpass-cli --bin gitpass -- account login --account octocat
```

**GitHub OAuth app:** create an app, set **Authorization callback URL** to exactly **`http://127.0.0.1:8765/callback`** (or set `--port` / env `GITPASS_OAUTH_PORT` and use the same URL here). The default scopes are `read:user read:org repo` (override with `--scopes` or `GITPASS_OAUTH_SCOPES`).

```bash
# Inside a clone, bind this repo to an account (uses `origin` to fill owner/repo)
cargo run -p gitpass-cli --bin gitpass -- repo bind --account octocat
# or: --account <uuid>
cargo run -p gitpass-cli --bin gitpass -- repo list

# Pin which repo the menu bar tray tooltip shows (GitPass desktop); `--path` optional
cargo run -p gitpass-cli --bin gitpass -- ui pin
cargo run -p gitpass-cli --bin gitpass -- ui unpin
```

Hooks update **`ui.last_active_repo_binding_id`** after each successful validation so the tray can follow your last activity when nothing is pinned.

Example shape (also editable by hand):

```toml
schema_version = 1

[[accounts]]
id = "00000000-0000-0000-0000-000000000000"
github_login = "octocat"
label = "Personal"
auth = "ssh"

[[repos]]
id = "00000000-0000-0000-0000-000000000001"
account_id = "00000000-0000-0000-0000-000000000000"
root_path = "/Users/you/src/my-repo"
github_owner = "octocat"
github_repo = "my-repo"

[ui]
```

## Build (desktop — Tauri)

From `gitpass-desktop/`:

```bash
npm install
npm run tauri dev
```

(Requires the Rust targets and Xcode CLI tools for the WebView on macOS.)

**Instalar como app (.app / .dmg)**, sem precisar do terminal depois do build:

```bash
cd gitpass-desktop && npm install && npm run tauri build
```

Saída típica (Apple Silicon):

- **Aplicação:** `target/release/bundle/macos/GitPass.app` — arrasta para **Aplicações** no Finder e abre com duplo clique.
- **DMG (opcional):** `target/release/bundle/dmg/GitPass_0.1.0_aarch64.dmg` — montar e arrastar o `.app` para Aplicações.

> Nota: a primeira abertura pode exigir **clicar com o botão direito → Abrir** (app não assinado pela Apple). Assinatura e notarização são passos separados, se quiseres distribuição sem avisos.

### OAuth “plug and play” (desktop + CLI)

- **Device flow (default for desktop):** only needs a **public OAuth App `client_id`**. No `client_secret`, no `oauth.env` for end users. In GitHub → **Developer settings** → your **OAuth App**, enable **Device flow**.
- **Ship the client id in the binary** (so users open the app and tap **Connect**):

  ```bash
  export GITPASS_GITHUB_OAUTH_CLIENT_ID="Ov23..."
  cd gitpass-desktop && npm run tauri build
  ```

  The `gitpass-core` crate embeds it at compile time. Users still **authorize once** in the browser (GitHub shows a short code — that’s normal).

- **Optional override:** `GITHUB_OAUTH_CLIENT_ID` alone (env or `oauth.env`) if you don’t embed at build time.

- **Web/PKCE flow (CLI or advanced):** if **`GITHUB_OAUTH_CLIENT_ID` and `GITHUB_OAUTH_CLIENT_SECRET`** are set, `gitpass account login` uses the localhost callback (`http://127.0.0.1:{port}/callback`); otherwise it falls back to **device flow** with the same `client_id` rules as above.

The UI shows **accounts**, **GitHub cloud repos** (per account, via API), **local bindings** (folder path + owner/repo), and **recent `git log`** for the selected binding. **Add local repo** opens a folder picker (macOS) and stores a binding from `origin`.

The app starts with the **main window hidden**; use the **menu bar tray icon** (template ring, generated at build) — **left-click** toggles the window, **right-click** opens a menu (Open / Hide / Quit). Closing the window **hides** to the tray (does not quit). The tray **tooltip** reflects **`config.toml`**: shows **@login** and repo **owner/name** when a single binding exists, or the **pinned** / **last active** binding when set (`ui.pinned_repo_binding_id` / `ui.last_active_repo_binding_id`).

## Status

**Current:** IPC, **`config.toml`**, CLI (`account`, `repo`, `config`, hooks, OAuth, credential helper), **daemon + GitHub API disambiguation**, **Tauri** tray + **desktop dashboard** (accounts, cloud repo list, local bindings, commit log). Next: Login Item for `gitpassd`, richer graph/history, polish.

## License

MIT
