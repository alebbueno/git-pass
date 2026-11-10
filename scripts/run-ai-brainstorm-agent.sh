#!/usr/bin/env bash
# Worktree + cursor-agent: brainstorming do plano docs/plans/ai-llm.md (sem código de IA).
set -euo pipefail

ROOT="/Volumes/Dados/Server/git-pass"
WT="/Volumes/Dados/Server/git-pass-wt/ai-brainstorm"
AGENTS="/tmp/gitpass-agents"
MODEL="${AI_BRAINSTORM_MODEL:-claude-opus-5-5-high}"
DEFAULT_AGENT="$HOME/.local/bin/cursor-agent"
LOG="$AGENTS/ai-brainstorm.log"
REPORT="$AGENTS/ai-brainstorm-report.md"
BRAINSTORM_SKILL="/Users/alessandro/.agents/skills/brainstorming/SKILL.md"
AGENT_DOC="$ROOT/docs/agents/ai-brainstorm.md"

agent_bin() {
  if [[ -n "${AI_BRAINSTORM_AGENT:-}" && -x "${AI_BRAINSTORM_AGENT}" ]]; then
    echo "${AI_BRAINSTORM_AGENT}"
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
    git -C "$WT" merge --abort 2>/dev/null || true
    git -C "$WT" reset --hard main
  fi
else
  if git show-ref --verify --quiet refs/heads/agent/ai-brainstorm; then
    git worktree add -q "$WT" agent/ai-brainstorm
  else
    git worktree add -q -b agent/ai-brainstorm "$WT" main
  fi
fi

mkdir -p "$WT/docs/agents" "$WT/scripts"
cp -f "$AGENT_DOC" "$WT/docs/agents/ai-brainstorm.md"
cp -f "$ROOT/docs/plans/ai-llm.md" "$WT/docs/plans/ai-llm.md" 2>/dev/null || mkdir -p "$WT/docs/plans" && cp -f "$ROOT/docs/plans/ai-llm.md" "$WT/docs/plans/"
cp -f "$ROOT/scripts/run-ai-brainstorm-agent.sh" "$WT/scripts/" 2>/dev/null || true
chmod +x "$WT/scripts/run-ai-brainstorm-agent.sh" 2>/dev/null || true

PROMPT=$(cat <<EOF
Você é o agente ai-brainstorm do GitPass.

1. Leia e siga RIGOROSAMENTE a skill brainstorming: ${BRAINSTORM_SKILL}
2. Leia o playbook: docs/agents/ai-brainstorm.md
3. Analise e melhore docs/plans/ai-llm.md (não implemente código).

Entregue:
- docs/plans/ai-llm-brainstorm-report.md (relatório completo + decision log)
- Atualize docs/plans/ai-llm.md com o design revisado
- Resumo executivo em ${REPORT}

Encerre sem implementar features de IA.
EOF
)

if AGENT_BIN="$(agent_bin)"; then
  (
    cd "$WT"
    "$AGENT_BIN" -p --force --trust --workspace "$WT" --model "$MODEL" "$PROMPT"
  ) >"$LOG" 2>&1 &
  echo "ai-brainstorm agent PID $! — log: $LOG"
  echo "Relatório esperado: $REPORT"
else
  echo "cursor-agent não encontrado. Instale ou defina AI_BRAINSTORM_AGENT." >&2
  exit 1
fi
