#!/usr/bin/env python3
"""
LLM Hotspot integration tests using the Anthropic client library.

Usage:
  python test_hotspot_anthropic.py --api-key hs_xxx
  LLMHOTSPOT_API_KEY=hs_xxx python test_hotspot_anthropic.py
  python test_hotspot_anthropic.py --url http://127.0.0.1:51824
"""

import argparse
import json
import os
import sys
import time
import traceback
from pathlib import Path

try:
    from anthropic import Anthropic, APIStatusError
except ImportError:
    print("ERROR: anthropic package not installed. Run: pip install anthropic")
    sys.exit(1)

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

GREEN = "\033[92m"
RED = "\033[91m"
CYAN = "\033[96m"
RESET = "\033[0m"
BOLD = "\033[1m"


def ok(msg):
    print(f"  {GREEN}OK{RESET} {msg}")


def info(msg):
    print(f"  {CYAN}i{RESET} {msg}")


RESULTS = []


def run_test(name, fn):
    print(f"\n{BOLD}{name}{RESET}")
    try:
        fn()
        RESULTS.append((name, True, ""))
    except AssertionError as e:
        RESULTS.append((name, False, str(e)))
        print(f"  {RED}ASSERTION FAILED{RESET}: {e}")
    except Exception as e:
        RESULTS.append((name, False, traceback.format_exc()))
        print(f"  {RED}EXCEPTION{RESET}: {type(e).__name__}: {e}")


def text_from_content(content):
    parts = []
    for block in content:
        if getattr(block, "type", "") == "text":
            txt = getattr(block, "text", "")
            if txt:
                parts.append(txt)
    return "\n".join(parts).strip()


TOOLS = [
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


def test_basic_message(client):
    resp = client.messages.create(
        model="claude-4.6-sonnet-medium",
        max_tokens=128,
        messages=[{"role": "user", "content": "Reply with exactly: HOTSPOT_OK"}],
    )
    info(f"id={resp.id} stop_reason={resp.stop_reason}")
    assert resp.type == "message", f"expected type=message, got {resp.type!r}"
    txt = text_from_content(resp.content)
    assert txt, "expected non-empty text response"
    ok(f"text={txt!r}")


def test_tool_use(client):
    resp = client.messages.create(
        model="claude-4.6-sonnet-medium",
        max_tokens=256,
        messages=[{"role": "user", "content": "What's the weather in Paris?"}],
        tools=TOOLS,
        tool_choice={"type": "any"},
    )
    info(f"stop_reason={resp.stop_reason}")
    assert resp.stop_reason == "tool_use", f"expected tool_use, got {resp.stop_reason!r}"
    tool_blocks = [b for b in resp.content if getattr(b, "type", "") == "tool_use"]
    assert tool_blocks, "expected tool_use block"
    tb = tool_blocks[0]
    ok(f"tool={tb.name} input={tb.input}")


def test_tool_result_roundtrip(client):
    first = client.messages.create(
        model="claude-4.6-sonnet-medium",
        max_tokens=256,
        messages=[{"role": "user", "content": "What's the weather in London right now?"}],
        tools=TOOLS,
        tool_choice={"type": "any"},
    )
    tool_blocks = [b for b in first.content if getattr(b, "type", "") == "tool_use"]
    assert tool_blocks, "expected tool_use in first turn"
    tb = tool_blocks[0]

    second = client.messages.create(
        model="claude-4.6-sonnet-medium",
        max_tokens=256,
        messages=[
            {"role": "user", "content": "What's the weather in London right now?"},
            {
                "role": "assistant",
                "content": [{"type": "tool_use", "id": tb.id, "name": tb.name, "input": tb.input}],
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": tb.id,
                        "content": json.dumps(
                            {"temperature": 12, "unit": "celsius", "condition": "cloudy"}
                        ),
                    }
                ],
            },
        ],
        tools=TOOLS,
        tool_choice={"type": "auto"},
    )
    assert second.stop_reason in ("end_turn", "stop_sequence"), (
        f"unexpected stop_reason: {second.stop_reason!r}"
    )
    txt = text_from_content(second.content)
    assert txt, "expected final text after tool_result"
    ok(f"final_text={txt!r}")


def test_invalid_model(client):
    try:
        client.messages.create(
            model="claude-imaginary-not-real",
            max_tokens=32,
            messages=[{"role": "user", "content": "hi"}],
        )
        raise AssertionError("expected APIStatusError for invalid model")
    except APIStatusError as e:
        info(f"status={e.status_code}")
        assert e.status_code in (400, 404, 502), f"expected 400/404/502, got {e.status_code}"
        ok("invalid model rejected")


def detect_url():
    candidates = [
        Path.home() / "AppData/Local/llmhotspot/runtime.json",
        Path("/c/Users")
        / os.environ.get("USERNAME", "")
        / "AppData/Local/llmhotspot/runtime.json",
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


def main():
    parser = argparse.ArgumentParser(description="Anthropic SDK integration tests for LLM Hotspot")
    parser.add_argument("--url", default=None, help="Base URL (default: auto-detect)")
    parser.add_argument(
        "--api-key",
        default=os.environ.get("LLMHOTSPOT_API_KEY", ""),
        help="LLM Hotspot API key (hs_...)",
    )
    parser.add_argument("--filter", default="", help="Run only tests whose name contains this value")
    args = parser.parse_args()

    base_url = (args.url or detect_url()).rstrip("/")
    api_key = args.api_key
    if not api_key:
        print("ERROR: missing API key. Pass --api-key or set LLMHOTSPOT_API_KEY")
        sys.exit(1)

    print(f"\n{BOLD}Anthropic SDK Compatibility Tests{RESET}")
    print(f"  URL: {base_url}")
    print(f"  Key: {api_key[:12]}{'*' * (len(api_key) - 12) if len(api_key) > 12 else ''}")

    client = Anthropic(api_key=api_key, base_url=base_url)

    tests = [
        ("Basic message", lambda: test_basic_message(client)),
        ("Tool use", lambda: test_tool_use(client)),
        ("Tool result roundtrip", lambda: test_tool_result_roundtrip(client)),
        ("Invalid model rejected", lambda: test_invalid_model(client)),
    ]
    if args.filter:
        tests = [(n, f) for n, f in tests if args.filter.lower() in n.lower()]
        if not tests:
            print(f"No tests matched filter {args.filter!r}")
            sys.exit(1)

    start = time.time()
    for name, fn in tests:
        run_test(name, fn)

    elapsed = time.time() - start
    passed = sum(1 for _, ok_, _ in RESULTS if ok_)
    failed = sum(1 for _, ok_, _ in RESULTS if not ok_)

    print(f"\n{'-' * 56}")
    print(f"Results: {passed} passed, {failed} failed ({elapsed:.1f}s)")
    print("-" * 56)
    if failed:
        for name, ok_, err in RESULTS:
            if not ok_:
                print(f"  FAIL {name}: {err.splitlines()[0] if err else 'unknown error'}")
        sys.exit(1)


if __name__ == "__main__":
    main()
