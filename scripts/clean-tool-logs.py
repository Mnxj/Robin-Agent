#!/usr/bin/env python3
"""
Normalize tool-call logs.

Features:
- Fixes mismatched closing tags in common cases:
  - inside <tool_response> ... </tool_call>  -> treated as </tool_response>
  - inside <tool_call> ... </tool_response>  -> treated as </tool_call>
- Auto-closes an open block when a new block starts.
- Drops empty <tool_response> blocks.
- Preserves non-tool text outside blocks.

Usage:
  python3 scripts/clean-tool-logs.py input.txt
  python3 scripts/clean-tool-logs.py input.txt -o output.txt
  python3 scripts/clean-tool-logs.py --in-place input.txt
  cat input.txt | python3 scripts/clean-tool-logs.py
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import List, Tuple


OPEN_CALL = "<tool_call>"
OPEN_RESP = "<tool_response>"
CLOSE_CALL = "</tool_call>"
CLOSE_RESP = "</tool_response>"


def _flush_block(
    out_blocks: List[Tuple[str, str]],
    state: str | None,
    buf: List[str],
) -> None:
    if not state:
        return
    content = "\n".join(buf).strip("\n")
    if state == "response" and not content.strip():
        return
    out_blocks.append((state, content))


def normalize(text: str) -> str:
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    blocks: List[Tuple[str, str]] = []  # ("text"|"call"|"response", content)
    state: str | None = None            # None|"call"|"response"
    buf: List[str] = []
    outside_buf: List[str] = []

    def flush_outside() -> None:
        nonlocal outside_buf
        if outside_buf:
            blocks.append(("text", "\n".join(outside_buf).strip("\n")))
            outside_buf = []

    for line in lines:
        tag = line.strip()

        if tag == OPEN_CALL:
            if state:
                _flush_block(blocks, state, buf)
            state = "call"
            buf = []
            flush_outside()
            continue

        if tag == OPEN_RESP:
            if state:
                _flush_block(blocks, state, buf)
            state = "response"
            buf = []
            flush_outside()
            continue

        if tag == CLOSE_CALL:
            if state == "call":
                _flush_block(blocks, "call", buf)
                state = None
                buf = []
            elif state == "response":
                # Mismatched close, treat as closing response.
                _flush_block(blocks, "response", buf)
                state = None
                buf = []
            else:
                # Stray close tag: ignore.
                pass
            continue

        if tag == CLOSE_RESP:
            if state == "response":
                _flush_block(blocks, "response", buf)
                state = None
                buf = []
            elif state == "call":
                # Mismatched close, treat as closing call.
                _flush_block(blocks, "call", buf)
                state = None
                buf = []
            else:
                # Stray close tag: ignore.
                pass
            continue

        if state:
            buf.append(line)
        else:
            outside_buf.append(line)

    if state:
        _flush_block(blocks, state, buf)
    flush_outside()

    rendered: List[str] = []
    for kind, content in blocks:
        if kind == "text":
            if content.strip():
                rendered.append(content.strip("\n"))
            continue
        if kind == "call":
            rendered.append(f"{OPEN_CALL}\n{content}\n{CLOSE_CALL}".strip("\n"))
        elif kind == "response":
            rendered.append(f"{OPEN_RESP}\n{content}\n{CLOSE_RESP}".strip("\n"))

    return "\n\n".join(part for part in rendered if part.strip()) + "\n"


def read_input(path: str | None) -> str:
    if path:
        return Path(path).read_text(encoding="utf-8")
    return sys.stdin.read()


def main() -> int:
    parser = argparse.ArgumentParser(description="Clean/normalize tool-call logs")
    parser.add_argument("input", nargs="?", help="input file (omit to read stdin)")
    parser.add_argument("-o", "--output", help="output file (default: stdout)")
    parser.add_argument(
        "--in-place",
        action="store_true",
        help="rewrite input file in place",
    )
    args = parser.parse_args()

    if args.in_place and not args.input:
        parser.error("--in-place requires an input file")
    if args.in_place and args.output:
        parser.error("--in-place cannot be used with --output")

    raw = read_input(args.input)
    cleaned = normalize(raw)

    if args.in_place:
        Path(args.input).write_text(cleaned, encoding="utf-8")
        return 0

    if args.output:
        Path(args.output).write_text(cleaned, encoding="utf-8")
        return 0

    sys.stdout.write(cleaned)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
