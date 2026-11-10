#!/usr/bin/env bash
# Instala o post-commit que dispara o agente build-gate quando o app muda.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_SRC="$ROOT/scripts/hooks/post-commit-build-gate"
GIT_HOOK="$ROOT/.git/hooks/post-commit"
MARKER="# gitpass-build-gate"

chmod +x "$HOOK_SRC" "$ROOT/scripts/run-build-gate-agent.sh" "$ROOT/scripts/verify-app-build.sh"

if [[ -f "$GIT_HOOK" ]] && grep -qF "$MARKER" "$GIT_HOOK"; then
  echo "Hook build-gate já instalado em .git/hooks/post-commit"
  exit 0
fi

if [[ -f "$GIT_HOOK" ]]; then
  cp "$GIT_HOOK" "$GIT_HOOK.bak.$(date +%s)"
  {
    cat "$GIT_HOOK"
    echo ""
    echo "$MARKER"
    echo "exec \"$HOOK_SRC\""
  } >"$GIT_HOOK.new"
  mv "$GIT_HOOK.new" "$GIT_HOOK"
else
  {
    echo "#!/usr/bin/env bash"
    echo "$MARKER"
    echo "exec \"$HOOK_SRC\""
  } >"$GIT_HOOK"
fi
chmod +x "$GIT_HOOK"
echo "Instalado: $GIT_HOOK (dispara build-gate após commit no app)"
