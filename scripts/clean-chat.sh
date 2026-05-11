#!/usr/bin/env bash
set -euo pipefail

# Wrapper for scripts/clean-tool-logs.py
#
# Usage:
#   ./scripts/clean-chat.sh file.txt
#   ./scripts/clean-chat.sh --in-place file.txt
#   ./scripts/clean-chat.sh --dir logs/
#   ./scripts/clean-chat.sh --dir logs/ --glob "*.log" --in-place

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PY_CLEANER="${ROOT_DIR}/scripts/clean-tool-logs.py"

if [[ ! -f "$PY_CLEANER" ]]; then
  echo "missing cleaner: $PY_CLEANER" >&2
  exit 1
fi

usage() {
  cat <<'EOF'
Usage:
  clean-chat.sh [--in-place] <file>
  clean-chat.sh --dir <directory> [--glob <pattern>] [--in-place]

Options:
  --in-place        Rewrite files in place.
  --dir <dir>       Batch process files in a directory.
  --glob <pattern>  File match pattern for --dir mode (default: *.txt).
  -h, --help        Show this help.
EOF
}

IN_PLACE=false
DIR_MODE=false
TARGET_FILE=""
TARGET_DIR=""
GLOB_PATTERN="*.txt"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --in-place)
      IN_PLACE=true
      shift
      ;;
    --dir)
      DIR_MODE=true
      TARGET_DIR="${2:-}"
      shift 2
      ;;
    --glob)
      GLOB_PATTERN="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      if [[ -z "$TARGET_FILE" ]]; then
        TARGET_FILE="$1"
      else
        echo "unexpected argument: $1" >&2
        usage
        exit 1
      fi
      shift
      ;;
  esac
done

if [[ "$DIR_MODE" == true ]]; then
  if [[ -z "$TARGET_DIR" ]]; then
    echo "--dir requires a directory path" >&2
    exit 1
  fi
  if [[ ! -d "$TARGET_DIR" ]]; then
    echo "directory not found: $TARGET_DIR" >&2
    exit 1
  fi

  mapfile -t files < <(find "$TARGET_DIR" -type f -name "$GLOB_PATTERN" | sort)
  if [[ ${#files[@]} -eq 0 ]]; then
    echo "no files matched: ${TARGET_DIR}/${GLOB_PATTERN}"
    exit 0
  fi

  for f in "${files[@]}"; do
    if [[ "$IN_PLACE" == true ]]; then
      python3 "$PY_CLEANER" --in-place "$f"
      echo "cleaned: $f"
    else
      out="${f%.*}.cleaned.${f##*.}"
      python3 "$PY_CLEANER" "$f" -o "$out"
      echo "cleaned -> $out"
    fi
  done
  exit 0
fi

if [[ -z "$TARGET_FILE" ]]; then
  usage
  exit 1
fi
if [[ ! -f "$TARGET_FILE" ]]; then
  echo "file not found: $TARGET_FILE" >&2
  exit 1
fi

if [[ "$IN_PLACE" == true ]]; then
  python3 "$PY_CLEANER" --in-place "$TARGET_FILE"
  echo "cleaned: $TARGET_FILE"
else
  python3 "$PY_CLEANER" "$TARGET_FILE"
fi
