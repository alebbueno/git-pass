#!/usr/bin/env bash
# Cria/atualiza o worktree build-gate e dispara o cursor-agent de verificação.
set -euo pipefail

ROOT="/Volumes/Dados/Server/git-pass"
WT="/Volumes/Dados/Server/git-pass-wt/build-gate"
AGENTS="/tmp/gitpass-agents"
MODEL="${BUILD_GATE_MODEL:-claude-opus-5-5-high}"
AGENT_BIN="${CURSOR_AGENT:-$HOME/.local/bin/cursor-agent}"
LOG="$AGENTS/build-gate.log"

mkdir -p "$AGENTS" "$(dirname "$WT")"

cd "$ROOT"
if git worktree list | grep -qF "$WT"; then
  git -C "$WT" fetch origin main 2>/dev/null || true
  git -C "$WT" merge -q main 2>/dev/null || git -C "$WT" reset --hard main
else
  git worktree add -q -b agent/build-gate "$WT" main
fi

chmod +x "$WT/scripts/verify-app-build.sh"

if [[ ! -x "$AGENT_BIN" ]]; then
  echo "cursor-agent não encontrado em $AGENT_BIN" >&2
  echo "Rode manualmente: cd $WT && ./scripts/verify-app-build.sh" >&2
  exit 1
fi

(
  cd "$WT"
  "$AGENT_BIN" -p --force --trust --workspace "$WT" --model "$MODEL" \
    "Leia docs/agents/build-gate.md, sincronize com main, rode ./scripts/verify-app-build.sh, escreva o relatório em /tmp/gitpass-agents/build-gate-report.md e encerre."
) >"$LOG" 2>&1 &
echo "build-gate agent PID $! — log: $LOG"
