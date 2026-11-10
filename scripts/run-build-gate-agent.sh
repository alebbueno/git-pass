#!/usr/bin/env bash
# Cria/atualiza o worktree build-gate e dispara o cursor-agent de verificação.
set -euo pipefail

ROOT="/Volumes/Dados/Server/git-pass"
WT="/Volumes/Dados/Server/git-pass-wt/build-gate"
AGENTS="/tmp/gitpass-agents"
MODEL="${BUILD_GATE_MODEL:-claude-opus-5-5-high}"
# Não use $CURSOR_AGENT: no shell do Cursor costuma ser "1" (flag), não o caminho do binário.
DEFAULT_AGENT="$HOME/.local/bin/cursor-agent"
LOG="$AGENTS/build-gate.log"

agent_bin() {
  if [[ -n "${BUILD_GATE_AGENT:-}" && -x "${BUILD_GATE_AGENT}" ]]; then
    echo "${BUILD_GATE_AGENT}"
    return 0
  fi
  if [[ -x "$DEFAULT_AGENT" ]]; then
    echo "$DEFAULT_AGENT"
    return 0
  fi
  command -v cursor-agent 2>/dev/null || return 1
}

mkdir -p "$AGENTS" "$(dirname "$WT")"

cd "$ROOT"
if git worktree list | grep -qF "$WT"; then
  git -C "$WT" fetch origin main 2>/dev/null || true
  if ! git -C "$WT" merge -q main 2>/dev/null; then
    git -C "$WT" reset --hard main
  fi
else
  if git show-ref --verify --quiet refs/heads/agent/build-gate; then
    git worktree add -q "$WT" agent/build-gate
  else
    git worktree add -q -b agent/build-gate "$WT" main
  fi
fi
# Scripts novos podem existir só no main antes do merge: garantir cópia mínima.
mkdir -p "$WT/scripts"
cp -f "$ROOT/scripts/verify-app-build.sh" "$WT/scripts/" 2>/dev/null || true
cp -f "$ROOT/scripts/run-build-gate-agent.sh" "$WT/scripts/" 2>/dev/null || true
chmod +x "$WT/scripts/"*.sh 2>/dev/null || true

chmod +x "$WT/scripts/verify-app-build.sh"

if AGENT_BIN="$(agent_bin)"; then
  (
    cd "$WT"
    "$AGENT_BIN" -p --force --trust --workspace "$WT" --model "$MODEL" \
      "Leia docs/agents/build-gate.md, sincronize com main, rode ./scripts/verify-app-build.sh, escreva o relatório em /tmp/gitpass-agents/build-gate-report.md e encerre."
  ) >"$LOG" 2>&1 &
  echo "build-gate agent PID $! — log: $LOG"
else
  (
    cd "$WT"
    echo "# build-gate (verify direto, sem cursor-agent)" > /tmp/gitpass-agents/build-gate-report.md
    echo "Commit: $(git rev-parse HEAD)" >> /tmp/gitpass-agents/build-gate-report.md
    ./scripts/verify-app-build.sh
    echo "Resultado: ok (exit $?)" >> /tmp/gitpass-agents/build-gate-report.md
  ) >"$LOG" 2>&1 &
  echo "build-gate verify PID $! (sem agente) — log: $LOG"
fi
