#!/usr/bin/env bash
set -euo pipefail

ROOT="/Volumes/Dados/Server/git-pass"
WT="/Volumes/Dados/Server/git-pass-wt/ai-brainstorm"
AGENTS="/tmp/gitpass-agents"
MODEL="${AI_MULTI_REVIEW_MODEL:-claude-opus-5-5-high}"
DEFAULT_AGENT="$HOME/.local/bin/cursor-agent"
LOG="$AGENTS/ai-multi-review.log"
REPORT="$AGENTS/ai-multi-review-report.md"
SKILL="/Users/alessandro/.agents/skills/multi-agent-brainstorming/SKILL.md"

agent_bin() {
  if [[ -n "${AI_MULTI_REVIEW_AGENT:-}" && -x "${AI_MULTI_REVIEW_AGENT}" ]]; then
    echo "${AI_MULTI_REVIEW_AGENT}"
    return 0
  fi
  if [[ -x "$DEFAULT_AGENT" ]]; then
    echo "$DEFAULT_AGENT"
    return 0
  fi
  command -v cursor-agent 2>/dev/null || return 1
}

mkdir -p "$AGENTS"
cd "$ROOT"
git -C "$WT" fetch origin main 2>/dev/null || true
git -C "$WT" merge -q main 2>/dev/null || git -C "$WT" reset --hard main

mkdir -p "$WT/docs/agents" "$WT/docs/plans" "$WT/scripts"
cp -f "$ROOT/docs/agents/ai-multi-review.md" "$WT/docs/agents/"
cp -f "$ROOT/docs/plans/ai-llm.md" "$ROOT/docs/plans/ai-llm-brainstorm-report.md" "$WT/docs/plans/" 2>/dev/null || true
cp -f "$ROOT/scripts/run-ai-multi-review-agent.sh" "$WT/scripts/" 2>/dev/null || true
chmod +x "$WT/scripts/run-ai-multi-review-agent.sh" 2>/dev/null || true

if ! AGENT_BIN="$(agent_bin)"; then
  echo "cursor-agent não encontrado." >&2
  exit 1
fi

cd "$WT"
"$AGENT_BIN" -p --force --trust --workspace "$WT" --model "$MODEL" "$(cat <<EOF
Siga docs/agents/ai-multi-review.md e a skill: ${SKILL}
O understanding lock e decisões 1-9 já foram confirmados pelo usuário.
Não implemente código. Produza os entregáveis listados.
Veredito explícito: GO Fase 0 | GO condicional | NO-GO.
Resumo em ${REPORT}
EOF
)" 2>&1 | tee "$LOG"
