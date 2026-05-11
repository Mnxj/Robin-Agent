#!/usr/bin/env bash
set -euo pipefail

# Smoke test for skills + memory wiring.
#
# What it verifies:
# 1) gateway health
# 2) /settings/api/tools includes load_skill + load_memory
# 3) skill upload/list/delete works
# 4) memory save/get/delete works
#
# Usage:
#   ./scripts/smoke-skill-memory.sh
#   ROBIN_BASE_URL=http://127.0.0.1:18789 ./scripts/smoke-skill-memory.sh
#   ROBIN_TOKEN=xxx ./scripts/smoke-skill-memory.sh

BASE_URL="${ROBIN_BASE_URL:-http://127.0.0.1:18789}"
TOKEN="${ROBIN_TOKEN:-}"
TMP_DIR="$(mktemp -d)"
# Ensure local data dirs exist for upload/save endpoints.
mkdir -p "${HOME}/.robin/skills" "${HOME}/.robin/memory/entries"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "[1/6] health check: ${BASE_URL}/health"
if ! curl -fsS "${BASE_URL}/health" >/dev/null; then
  echo "gateway unavailable at ${BASE_URL}" >&2
  exit 1
fi

curl_json() {
  if [[ -n "$TOKEN" ]]; then
    curl -fsS -H "Authorization: Bearer ${TOKEN}" "$@"
  else
    curl -fsS "$@"
  fi
}

echo "[2/6] checking tools endpoint"
tools_json="$(curl_json "${BASE_URL}/settings/api/tools")"
python3 - "$tools_json" <<'PY'
import json, sys
obj = json.loads(sys.argv[1])
tools = {t.get("name", "") for t in obj.get("tools", [])}
missing = [x for x in ("load_skill", "load_memory") if x not in tools]
if missing:
    raise SystemExit(f"missing tools: {missing}")
print("tools ok:", ", ".join(sorted([t for t in tools if t in {"load_skill","load_memory"}])))
PY

skill_name="smoke-skill-$(date +%s).md"
skill_path="${TMP_DIR}/${skill_name}"
cat >"$skill_path" <<'EOF'
---
name: smoke-skill
description: skill smoke test
tags: [smoke, test]
---

## Smoke

This is a smoke skill body.
EOF

echo "[3/6] uploading skill: ${skill_name}"
curl_json \
  -F "file=@${skill_path};type=text/markdown" \
  "${BASE_URL}/settings/api/skills" >/dev/null

echo "[4/6] validating skill appears in list"
skills_json="$(curl_json "${BASE_URL}/settings/api/skills")"
python3 - "$skills_json" "$skill_name" <<'PY'
import json, sys
obj = json.loads(sys.argv[1])
filename = sys.argv[2]
files = {s.get("filename","") for s in obj.get("skills", [])}
if filename not in files:
    raise SystemExit(f"uploaded skill missing in list: {filename}")
print("skill list ok")
PY

echo "[5/6] deleting skill"
curl_json -X DELETE "${BASE_URL}/settings/api/skills/${skill_name}" >/dev/null

mem_id="smoke-memory-$(date +%s)"
mem_body="{
  \"id\": \"${mem_id}\",
  \"content\": \"# Smoke Memory\\nThis is a smoke memory entry.\"
}"

echo "[6/6] memory save/get/delete"
curl_json \
  -H "Content-Type: application/json" \
  -d "$mem_body" \
  "${BASE_URL}/settings/api/memory" >/dev/null

got="$(curl_json "${BASE_URL}/settings/api/memory/${mem_id}")"
if [[ "$got" != *"Smoke Memory"* ]]; then
  echo "memory get failed: unexpected content" >&2
  exit 1
fi

curl_json -X DELETE "${BASE_URL}/settings/api/memory/${mem_id}" >/dev/null

echo "smoke test passed: skills + memory wiring is functional"
