#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
LAB_ROOT="${CODEX_MITM_LAB_ROOT:-$REPO_ROOT/infra/codex-mitm-lab}"
BASE_URL="${CODEX_LAB_BASE_URL:-https://ai.input.im}"
CONTAINER_NAME="${CODEX_MITM_CONTAINER_NAME:-codex-gateway-mitm-lab}"
COMPOSE_PROJECT_NAME="${CODEX_MITM_COMPOSE_PROJECT:-codex-gateway-mitm}"

usage() {
  cat <<'EOF'
Usage: scripts/codex-mitm-lab.sh <command>

Commands:
  sync      Copy local Codex auth/config into the MITM lab and set base_url.
  up        Sync config, build, and start the transparent MITM lab.
  down      Stop the MITM lab container.
  restart   Recreate the MITM lab container.
  shell     Enter the lab as the codexlab user.
  logs      Follow mitmproxy logs.
  analyze   Summarize captured flows with secret/prompt redaction.

Environment:
  CODEX_LAB_BASE_URL     Defaults to https://ai.input.im.
  CODEX_MITM_LAB_ROOT    Defaults to ./infra/codex-mitm-lab.
  CODEX_SOURCE_HOME      Defaults to ~/.codex.
  CODEX_VERSION          Defaults to 0.142.5.
  CODEX_MITM_COMPOSE_PROJECT
                           Defaults to codex-gateway-mitm.
  MITM_BLOCK_HOSTS       Optional host list forwarded to docker compose.
EOF
}

compose() {
  docker compose -p "$COMPOSE_PROJECT_NAME" --project-directory "$LAB_ROOT" -f "$LAB_ROOT/docker-compose.yml" "$@"
}

sync_config() {
  CODEX_LAB_BASE_URL="$BASE_URL" python3 "$LAB_ROOT/scripts/sync-codex-config.py"
}

case "${1:-}" in
  sync)
    sync_config
    ;;
  up)
    sync_config
    compose up -d --build
    ;;
  down)
    compose down
    ;;
  restart)
    sync_config
    compose up -d --build --force-recreate
    ;;
  shell)
    docker exec -it "$CONTAINER_NAME" su - codexlab
    ;;
  logs)
    compose logs -f codex-mitm
    ;;
  analyze)
    shift
    docker exec -i "$CONTAINER_NAME" python3 - /flows/codex.mitm "$@" < "$SCRIPT_DIR/analyze-codex-mitm-flows.py"
    ;;
  ""|-h|--help|help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
