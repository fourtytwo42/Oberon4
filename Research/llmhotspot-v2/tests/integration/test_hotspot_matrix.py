#!/usr/bin/env python3
"""
LLM Hotspot matrix integration tests.

Covers both API standards:
  - OpenAI standard (/v1/chat/completions via openai SDK)
  - Anthropic standard (/v1/messages via anthropic SDK)

For each standard, tests matrix dimensions:
  - streaming: non-streaming + streaming
  - tools: with tools + without tools
  - context size: small + large

Usage:
  python test_hotspot_matrix.py --api-key hs_xxx
  LLMHOTSPOT_API_KEY=hs_xxx python test_hotspot_matrix.py
  python test_hotspot_matrix.py --url http://127.0.0.1:51824
"""

import argparse
import json
import os
import sys
import time
import traceback
from dataclasses import dataclass
from pathlib import Path

try:
    from openai import OpenAI
    import openai as openai_module
except ImportError:
    print("ERROR: openai package not installed. Run: pip install openai")
    sys.exit(1)

try:
    from anthropic import Anthropic
except ImportError:
    print("ERROR: anthropic package not installed. Run: pip install anthropic")
    sys.exit(1)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

GREEN = "\033[92m"
RED = "\033[91m"
YELLOW = "\033[93m"
CYAN = "\033[96m"
RESET = "\033[0m"
BOLD = "\033[1m"


def info(msg):
    print(f"{CYAN}{msg}{RESET}")


@dataclass
class MatrixCase:
    standard: str
    streaming: bool
    tools: bool
    context_size: str  # small | large

    def name(self) -> str:
        s = "stream" if self.streaming else "nonstream"
        t = "tools" if self.tools else "no-tools"
        return f"{self.standard} | {s} | {t} | {self.context_size}"


OPENAI_TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get weather for a city.",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {"type": "string"},
                    "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]},
                },
                "required": ["location"],
            },
        },
    }
]

ANTHROPIC_TOOLS = [
    {
        "name": "get_weather",
        "description": "Get weather for a city.",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": {"type": "string"},
                "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]},
            },
            "required": ["location"],
        },
    }
]


def detect_url() -> str:
    candidates = [
        Path.home() / "AppData/Local/llmhotspot/runtime.json",
        Path("/c/Users") / os.environ.get("USERNAME", "") / "AppData/Local/llmhotspot/runtime.json",
    ]
    for p in candidates:
        if p.exists():
            try:
                data = json.loads(p.read_text())
                url = data.get("local_url", "")
                if url:
                    return url.rstrip("/")
            except Exception:
                pass
    return "http://127.0.0.1:51824"


def make_prompt(case: MatrixCase, large_chars: int) -> str:
    if case.context_size == "small":
        if case.tools:
            return "Use get_weather for Paris and return the tool call."
        return "Reply with exactly: MATRIX_OK"
    filler = ("0123456789abcdef" * ((large_chars // 16) + 2))[:large_chars]
    if case.tools:
        return (
            "Large context attached. Ignore filler except to acknowledge receipt. "
            "Use get_weather for Paris. "
            f"FILLER:{filler}"
        )
    return (
        "Large context attached. Ignore filler and reply with exactly: MATRIX_LARGE_OK. "
        f"FILLER:{filler}"
    )


def run_openai_case(client: OpenAI, case: MatrixCase, large_chars: int) -> None:
    prompt = make_prompt(case, large_chars)
    payload = {
        "model": "gpt-5.3-codex",
        "messages": [{"role": "user", "content": prompt}],
        "stream": case.streaming,
        "max_tokens": 256,
    }
    if case.tools:
        payload["tools"] = OPENAI_TOOLS
        payload["tool_choice"] = "required"

    if not case.streaming:
        resp = client.chat.completions.create(**payload)
        choice = resp.choices[0]
        if case.tools:
            assert choice.finish_reason == "tool_calls", f"expected tool_calls, got {choice.finish_reason!r}"
            assert choice.message.tool_calls, "expected tool_calls in message"
        else:
            assert choice.finish_reason == "stop", f"expected stop, got {choice.finish_reason!r}"
            assert (choice.message.content or "").strip(), "expected non-empty content"
        return

    chunks = list(client.chat.completions.create(**payload))
    assert chunks, "expected chunks"
    finish_chunks = [c for c in chunks if c.choices and c.choices[0].finish_reason]
    assert finish_chunks, "expected finish chunk"
    final_reason = finish_chunks[-1].choices[0].finish_reason
    if case.tools:
        assert final_reason == "tool_calls", f"expected tool_calls, got {final_reason!r}"
        tc_chunks = [c for c in chunks if c.choices and c.choices[0].delta.tool_calls]
        assert tc_chunks, "expected streamed tool_calls"
    else:
        assert final_reason == "stop", f"expected stop, got {final_reason!r}"
        text = "".join(
            c.choices[0].delta.content or ""
            for c in chunks
            if c.choices and c.choices[0].delta.content
        )
        assert text.strip(), "expected streamed text content"


def run_anthropic_case(client: Anthropic, case: MatrixCase, large_chars: int) -> None:
    prompt = make_prompt(case, large_chars)
    messages = [{"role": "user", "content": prompt}]
    model = "claude-4.6-sonnet-medium"

    kwargs = {
        "model": model,
        "max_tokens": 256,
        "messages": messages,
    }
    if case.tools:
        kwargs["tools"] = ANTHROPIC_TOOLS
        kwargs["tool_choice"] = {"type": "any"}

    if not case.streaming:
        resp = client.messages.create(**kwargs)
        if case.tools:
            assert resp.stop_reason == "tool_use", f"expected tool_use, got {resp.stop_reason!r}"
            blocks = [b for b in resp.content if getattr(b, "type", "") == "tool_use"]
            assert blocks, "expected tool_use blocks"
        else:
            text_blocks = [b.text for b in resp.content if getattr(b, "type", "") == "text" and getattr(b, "text", "")]
            assert text_blocks, "expected text response"
        return

    # Anthropic standard streaming.
    # We assert we receive at least one text delta or tool_use event and a final stop.
    with client.messages.stream(**kwargs) as stream:
        text = ""
        saw_tool = False
        for event in stream:
            et = getattr(event, "type", "")
            if et == "content_block_delta":
                delta = getattr(event, "delta", None)
                if getattr(delta, "type", "") == "text_delta":
                    text += getattr(delta, "text", "")
            if et == "content_block_start":
                block = getattr(event, "content_block", None)
                if getattr(block, "type", "") == "tool_use":
                    saw_tool = True
        final = stream.get_final_message()
    if case.tools:
        assert final.stop_reason == "tool_use", f"expected tool_use, got {final.stop_reason!r}"
        assert saw_tool, "expected streamed tool_use"
    else:
        assert final.stop_reason in ("end_turn", "stop_sequence"), (
            f"expected end_turn/stop_sequence, got {final.stop_reason!r}"
        )
        assert text.strip() or any(
            getattr(b, "type", "") == "text" and getattr(b, "text", "").strip() for b in final.content
        ), "expected streamed/final text"


def all_cases() -> list[MatrixCase]:
    cases: list[MatrixCase] = []
    for standard in ("openai", "anthropic"):
        for streaming in (False, True):
            for tools in (False, True):
                for context_size in ("small", "large"):
                    cases.append(
                        MatrixCase(
                            standard=standard,
                            streaming=streaming,
                            tools=tools,
                            context_size=context_size,
                        )
                    )
    return cases


def main():
    parser = argparse.ArgumentParser(description="LLM Hotspot robust matrix tests")
    parser.add_argument("--url", default=None, help="Base URL (default: auto-detect)")
    parser.add_argument("--api-key", default=os.environ.get("LLMHOTSPOT_API_KEY", ""), help="API key")
    parser.add_argument("--filter", default="", help="Substring filter on case names")
    parser.add_argument(
        "--large-context-chars",
        type=int,
        default=int(os.environ.get("LLMHOTSPOT_LARGE_CONTEXT_CHARS", "24000")),
        help="Large context payload size in characters",
    )
    args = parser.parse_args()

    base_url = (args.url or detect_url()).rstrip("/")
    api_key = args.api_key
    if not api_key:
        print("ERROR: missing API key (pass --api-key or set LLMHOTSPOT_API_KEY)")
        sys.exit(1)

    openai_client = OpenAI(api_key=api_key, base_url=f"{base_url}/v1")
    anthropic_client = Anthropic(api_key=api_key, base_url=base_url)

    print(f"\n{BOLD}LLM Hotspot Matrix Tests{RESET}")
    print(f"  URL: {base_url}")
    print(f"  Key: {api_key[:12]}{'*' * (len(api_key) - 12) if len(api_key) > 12 else ''}")
    print(f"  Large context chars: {args.large_context_chars}")

    cases = all_cases()
    if args.filter:
        cases = [c for c in cases if args.filter.lower() in c.name().lower()]
        if not cases:
            print(f"{YELLOW}No cases matched filter {args.filter!r}{RESET}")
            sys.exit(1)

    results: list[tuple[MatrixCase, bool, str]] = []
    start = time.time()
    for case in cases:
        print(f"\n{BOLD}{case.name()}{RESET}")
        try:
            if case.standard == "openai":
                run_openai_case(openai_client, case, args.large_context_chars)
            else:
                run_anthropic_case(anthropic_client, case, args.large_context_chars)
            print(f"  {GREEN}PASS{RESET}")
            results.append((case, True, ""))
        except Exception as e:
            msg = f"{type(e).__name__}: {e}"
            print(f"  {RED}FAIL{RESET} {msg}")
            results.append((case, False, traceback.format_exc()))

    elapsed = time.time() - start
    passed = sum(1 for _, ok, _ in results if ok)
    failed = len(results) - passed

    print(f"\n{'=' * 72}")
    print(f"{BOLD}Summary{RESET}: {GREEN}{passed} passed{RESET}, {RED}{failed} failed{RESET} ({elapsed:.1f}s)")
    print("=" * 72)

    if failed:
        print(f"\n{RED}Failed Cases:{RESET}")
        for case, ok, err in results:
            if ok:
                continue
            first = err.splitlines()[0] if err else "unknown error"
            print(f"  - {case.name()} :: {first}")
        sys.exit(1)

    print(f"\n{GREEN}{BOLD}All matrix cases passed.{RESET}")


if __name__ == "__main__":
    main()

