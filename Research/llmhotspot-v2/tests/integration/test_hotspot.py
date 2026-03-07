#!/usr/bin/env python3
"""
LLM Hotspot integration tests using the OpenAI client library.

Covers:
  - /v1/models list
  - Basic chat completion (non-streaming and streaming)
  - Tool calls with no tool_choice (tests null→required default fix)
  - Tool calls with explicit tool_choice values
  - Streaming tool calls (SSE chunk format)
  - Multi-turn agentic loop (tool_call → tool_result → final answer)
  - Parallel tool calls
  - reasoning_effort passthrough
  - Auth rejection (missing/bad token)
  - Invalid model rejection

Usage:
    python test_hotspot.py --api-key hs_xxx
    LLMHOTSPOT_API_KEY=hs_xxx python test_hotspot.py
    python test_hotspot.py --api-key hs_xxx --url http://127.0.0.1:51824
    python test_hotspot.py --api-key hs_xxx --filter tool   # run only tests whose name contains "tool"
"""

import os
import sys
import json
import time
import argparse
import traceback
from pathlib import Path

# Force UTF-8 output on Windows so Unicode symbols render correctly.
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

try:
    from openai import OpenAI
    import openai as openai_module
except ImportError:
    print("ERROR: openai package not installed. Run: pip install openai")
    sys.exit(1)

# ---------------------------------------------------------------------------
# Colour helpers
# ---------------------------------------------------------------------------
GREEN  = "\033[92m"
RED    = "\033[91m"
YELLOW = "\033[93m"
CYAN   = "\033[96m"
RESET  = "\033[0m"
BOLD   = "\033[1m"

def ok(msg):   print(f"  {GREEN}✓{RESET} {msg}")
def fail(msg): print(f"  {RED}✗{RESET} {msg}")
def info(msg): print(f"  {CYAN}·{RESET} {msg}")

# ---------------------------------------------------------------------------
# Test runner
# ---------------------------------------------------------------------------
RESULTS: list[tuple[str, bool, str]] = []

def run_test(name: str, fn):
    print(f"\n{BOLD}{name}{RESET}")
    try:
        fn()
        RESULTS.append((name, True, ""))
    except AssertionError as e:
        fail(f"ASSERTION FAILED: {e}")
        RESULTS.append((name, False, str(e)))
    except Exception as e:
        fail(f"EXCEPTION: {type(e).__name__}: {e}")
        RESULTS.append((name, False, traceback.format_exc()))

def assert_eq(label, actual, expected):
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")
    ok(f"{label} = {actual!r}")

def assert_in(label, value, collection):
    if value not in collection:
        raise AssertionError(f"{label}: {value!r} not in {collection!r}")
    ok(f"{label} {value!r} found")

def assert_not_in(label, value, collection):
    if value in collection:
        raise AssertionError(f"{label}: {value!r} unexpectedly present")
    ok(f"{label} {value!r} absent (correct)")

def assert_truthy(label, value):
    if not value:
        raise AssertionError(f"{label}: expected truthy, got {value!r}")
    ok(f"{label} is set")

def assert_isinstance(label, value, typ):
    if not isinstance(value, typ):
        raise AssertionError(f"{label}: expected {typ.__name__}, got {type(value).__name__}: {value!r}")
    ok(f"{label} is {typ.__name__}")

# ---------------------------------------------------------------------------
# Shared tool definitions (Cursor-style, with verbose descriptions to also
# verify our compact_params stripping doesn't break the call round-trip)
# ---------------------------------------------------------------------------
TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather for a given location. Returns temperature and conditions.",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "The city and country, e.g. 'London, UK'. Must be a real place."
                    },
                    "unit": {
                        "type": "string",
                        "enum": ["celsius", "fahrenheit"],
                        "description": "Temperature unit to use in the response. Defaults to celsius."
                    }
                },
                "required": ["location"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "search_web",
            "description": "Search the web for information. Use this when you need up-to-date information.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query string."
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "Number of results to return. Defaults to 5."
                    }
                },
                "required": ["query"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "run_code",
            "description": "Execute a shell command or code snippet and return the output.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command or code to execute."
                    },
                    "language": {
                        "type": "string",
                        "enum": ["bash", "python", "javascript"],
                        "description": "The language/shell to use."
                    }
                },
                "required": ["command", "language"]
            }
        }
    }
]

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_models_list(client):
    """GET /v1/models returns expected Codex models and NOT gpt-4o aliases."""
    models_page = client.models.list()
    ids = [m.id for m in models_page.data]
    info(f"Models returned: {ids}")

    assert_in("gpt-5.3-codex",      "gpt-5.3-codex",      ids)
    assert_in("gpt-5.1-codex-mini", "gpt-5.1-codex-mini", ids)
    assert_not_in("gpt-4o removed",      "gpt-4o",      ids)
    assert_not_in("gpt-4o-mini removed", "gpt-4o-mini", ids)


def test_basic_chat_non_streaming(client):
    """Simple non-streaming chat completion returns a valid response."""
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[{"role": "user", "content": "Reply with exactly: HOTSPOT_OK"}],
        stream=False,
    )
    info(f"id={resp.id}  model={resp.model}  finish={resp.choices[0].finish_reason}")

    assert_truthy("response id",         resp.id)
    assert_eq("object type",             resp.object, "chat.completion")
    assert_isinstance("choices list",    resp.choices, list)
    assert_eq("choice count",            len(resp.choices), 1)
    assert_eq("finish_reason",           resp.choices[0].finish_reason, "stop")
    assert_eq("message role",            resp.choices[0].message.role, "assistant")
    assert_truthy("message content",     resp.choices[0].message.content)
    info(f"Content: {resp.choices[0].message.content!r}")


def test_streaming_chat(client):
    """Streaming chat completion delivers properly formed SSE chunks."""
    chunks = list(client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[{"role": "user", "content": "Count to 3, one number per line."}],
        stream=True,
    ))

    info(f"Received {len(chunks)} chunks")
    assert_truthy("got chunks", len(chunks) > 0)

    # Every chunk must have correct object type
    for c in chunks:
        assert_eq("chunk object", c.object, "chat.completion.chunk")

    # Last non-empty choice chunk should have finish_reason=stop
    finish_chunks = [c for c in chunks if c.choices and c.choices[0].finish_reason]
    assert_truthy("has finish chunk", finish_chunks)
    assert_eq("final finish_reason", finish_chunks[-1].choices[0].finish_reason, "stop")

    # Concatenated content should be non-empty
    content = "".join(
        c.choices[0].delta.content or ""
        for c in chunks
        if c.choices and c.choices[0].delta.content
    )
    assert_truthy("streamed content", content.strip())
    info(f"Full content: {content.strip()!r}")


def test_tool_call_no_tool_choice(client):
    """
    When no tool_choice is sent, proxy should default to 'required' and the
    model must return a tool call (tests our null→required fix).
    """
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[
            {"role": "user", "content": "What's the weather in Paris?"}
        ],
        tools=TOOLS,
        # Intentionally omit tool_choice — proxy should inject "required"
        stream=False,
    )
    info(f"finish_reason={resp.choices[0].finish_reason}")

    assert_eq("finish_reason is tool_calls", resp.choices[0].finish_reason, "tool_calls")
    assert_truthy("tool_calls present",      resp.choices[0].message.tool_calls)

    tc = resp.choices[0].message.tool_calls[0]
    info(f"Tool called: {tc.function.name}({tc.function.arguments})")
    assert_eq("tool type",         tc.type, "function")
    assert_truthy("tool call id",  tc.id)
    assert_truthy("function name", tc.function.name)

    # Arguments must be valid JSON
    args = json.loads(tc.function.arguments)
    assert_isinstance("arguments is dict", args, dict)
    info(f"Parsed args: {args}")


def test_tool_call_explicit_required(client):
    """tool_choice='required' explicitly forces a tool call."""
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[
            {"role": "user", "content": "Search for the latest news about AI."}
        ],
        tools=TOOLS,
        tool_choice="required",
        stream=False,
    )
    assert_eq("finish_reason", resp.choices[0].finish_reason, "tool_calls")
    assert_truthy("tool_calls", resp.choices[0].message.tool_calls)

    tc = resp.choices[0].message.tool_calls[0]
    info(f"Tool: {tc.function.name}  args: {tc.function.arguments}")
    assert_truthy("function name", tc.function.name)
    json.loads(tc.function.arguments)  # must be valid JSON
    ok("arguments are valid JSON")


def test_tool_call_explicit_none(client):
    """tool_choice='none' prevents tool calls even when tools are provided."""
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[
            {"role": "user", "content": "What's the weather in Tokyo?"}
        ],
        tools=TOOLS,
        tool_choice="none",
        stream=False,
    )
    info(f"finish_reason={resp.choices[0].finish_reason}")
    assert_eq("finish_reason is stop", resp.choices[0].finish_reason, "stop")
    no_tool_calls = not resp.choices[0].message.tool_calls
    assert_truthy("no tool_calls returned", no_tool_calls)
    info(f"Content: {resp.choices[0].message.content!r}")


def test_tool_call_specific_function(client):
    """tool_choice={type:'function', function:{name:'...'}} forces a specific tool."""
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[
            {"role": "user", "content": "Do something useful for me."}
        ],
        tools=TOOLS,
        tool_choice={"type": "function", "function": {"name": "run_code"}},
        stream=False,
    )
    assert_eq("finish_reason", resp.choices[0].finish_reason, "tool_calls")
    tc = resp.choices[0].message.tool_calls[0]
    assert_eq("forced tool name", tc.function.name, "run_code")
    info(f"run_code args: {tc.function.arguments}")


def test_streaming_tool_calls(client):
    """Tool calls arrive correctly in SSE streaming mode."""
    chunks = list(client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[{"role": "user", "content": "What's the weather in Berlin?"}],
        tools=TOOLS,
        tool_choice="required",
        stream=True,
    ))

    info(f"Received {len(chunks)} chunks")
    assert_truthy("got chunks", len(chunks) > 0)

    # Find the finish chunk
    finish_chunks = [c for c in chunks if c.choices and c.choices[0].finish_reason]
    assert_truthy("has finish chunk", finish_chunks)
    assert_eq("finish_reason", finish_chunks[-1].choices[0].finish_reason, "tool_calls")

    # Accumulate tool call deltas into a single call
    tool_call_chunks = [
        c for c in chunks
        if c.choices and c.choices[0].delta.tool_calls
    ]
    assert_truthy("tool call chunks present", tool_call_chunks)

    # Reconstruct arguments from deltas
    all_args = "".join(
        tc.function.arguments or ""
        for c in tool_call_chunks
        for tc in (c.choices[0].delta.tool_calls or [])
        if tc.function and tc.function.arguments is not None
    )
    # Get tool name from first chunk that has it
    tool_name = next(
        (tc.function.name
         for c in tool_call_chunks
         for tc in (c.choices[0].delta.tool_calls or [])
         if tc.function and tc.function.name),
        None
    )
    info(f"Streamed tool: {tool_name}  args: {all_args!r}")
    assert_truthy("tool name", tool_name)
    json.loads(all_args)
    ok("streamed arguments are valid JSON")


def test_multi_turn_tool_loop(client):
    """
    Full agentic loop:
      1. User asks question → model calls get_weather
      2. We inject a fake tool result → model gives final text answer
    Verifies that tool_call history + function_call_output round-trips correctly.
    """
    # --- Turn 1: model calls a tool ---
    resp1 = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[
            {"role": "user", "content": "What's the weather in London right now?"}
        ],
        tools=TOOLS,
        tool_choice="required",
        stream=False,
    )
    assert_eq("turn1 finish_reason", resp1.choices[0].finish_reason, "tool_calls")
    tc = resp1.choices[0].message.tool_calls[0]
    info(f"Turn 1 tool call: {tc.function.name}({tc.function.arguments})")

    # --- Turn 2: inject tool result, get final answer ---
    fake_result = json.dumps({"temperature": 12, "unit": "celsius", "condition": "cloudy"})
    resp2 = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[
            {"role": "user",      "content": "What's the weather in London right now?"},
            resp1.choices[0].message,   # assistant message with tool_calls
            {
                "role":         "tool",
                "tool_call_id": tc.id,
                "name":         tc.function.name,
                "content":      fake_result,
            },
        ],
        tools=TOOLS,
        # Use "auto" so the model can choose to respond with text now that
        # it has the tool result, rather than being forced to call again.
        tool_choice="auto",
        stream=False,
    )
    info(f"Turn 2 finish_reason={resp2.choices[0].finish_reason}")
    info(f"Turn 2 content: {resp2.choices[0].message.content!r}")

    assert_eq("turn2 finish_reason", resp2.choices[0].finish_reason, "stop")
    assert_truthy("turn2 content",   resp2.choices[0].message.content)


def test_reasoning_effort_passthrough(client):
    """reasoning_effort (Chat Completions style) is accepted without error."""
    # We can't verify it changed behaviour, but it must not be rejected.
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[{"role": "user", "content": "Reply with: REASONING_OK"}],
        extra_body={"reasoning_effort": "low"},
        stream=False,
    )
    assert_eq("finish_reason", resp.choices[0].finish_reason, "stop")
    assert_truthy("content",   resp.choices[0].message.content)
    info(f"With reasoning_effort=low: {resp.choices[0].message.content!r}")


def test_reasoning_effort_responses_style(client):
    """reasoning.effort (Responses API style) is also accepted without error."""
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[{"role": "user", "content": "Reply with: REASONING_OK"}],
        extra_body={"reasoning": {"effort": "high"}},
        stream=False,
    )
    assert_eq("finish_reason", resp.choices[0].finish_reason, "stop")
    assert_truthy("content",   resp.choices[0].message.content)
    info(f"With reasoning.effort=high: {resp.choices[0].message.content!r}")


def test_auth_rejection_missing_token(base_url):
    """Requests without a bearer token are rejected with 401."""
    import urllib.request, urllib.error
    req = urllib.request.Request(
        f"{base_url}/v1/chat/completions",
        data=json.dumps({
            "model": "gpt-5.3-codex",
            "messages": [{"role": "user", "content": "hi"}],
        }).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        urllib.request.urlopen(req, timeout=10)
        raise AssertionError("Expected HTTP error, got 200")
    except urllib.error.HTTPError as e:
        info(f"Got HTTP {e.code} (expected 401 or 403)")
        assert_truthy("is auth error", e.code in (401, 403))
        body = json.loads(e.read())
        assert_truthy("error field", "error" in body)
        ok(f"error.type = {body['error'].get('type')!r}")


def test_auth_rejection_bad_token(base_url):
    """Requests with a clearly invalid bearer token are rejected."""
    import urllib.request, urllib.error
    req = urllib.request.Request(
        f"{base_url}/v1/chat/completions",
        data=json.dumps({
            "model": "gpt-5.3-codex",
            "messages": [{"role": "user", "content": "hi"}],
        }).encode(),
        headers={
            "Content-Type": "application/json",
            "Authorization": "Bearer totally_invalid_key_xyz123",
        },
        method="POST",
    )
    try:
        urllib.request.urlopen(req, timeout=10)
        raise AssertionError("Expected HTTP error, got 200")
    except urllib.error.HTTPError as e:
        info(f"Got HTTP {e.code}")
        assert_truthy("is auth error", e.code in (401, 403))
        ok("bad token correctly rejected")


def test_invalid_model_rejected(client):
    """Requesting a model not in the list returns a 404/400 error."""
    try:
        client.chat.completions.create(
            model="gpt-4-turbo-imaginary",
            messages=[{"role": "user", "content": "hi"}],
            stream=False,
        )
        raise AssertionError("Expected API error, got success")
    except openai_module.APIStatusError as e:
        info(f"Got HTTP {e.status_code}: {e.message!r}")
        assert_truthy("is client error", e.status_code in (400, 404))
        ok("unknown model correctly rejected")


def test_compact_schema_does_not_break_tool_calls(client):
    """
    Verifies that our compact_params schema stripping (removing property
    descriptions) doesn't prevent the model from calling tools correctly.
    Uses a tool with many properties and verbose descriptions.
    """
    verbose_tools = [
        {
            "type": "function",
            "function": {
                "name": "create_file",
                "description": "Create a new file at the given path with the given content.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "The absolute or relative file path where the file should be created. Must be a valid filesystem path.",
                        },
                        "content": {
                            "type": "string",
                            "description": "The text content to write into the file. May be empty for blank files.",
                        },
                        "overwrite": {
                            "type": "boolean",
                            "description": "If true, overwrite existing files. If false, fail if the file already exists. Defaults to false.",
                        },
                        "encoding": {
                            "type": "string",
                            "enum": ["utf-8", "utf-16", "ascii"],
                            "description": "The character encoding to use when writing the file.",
                        },
                        "permissions": {
                            "type": "string",
                            "description": "Unix-style permissions string e.g. '644'. Only applies on Unix systems.",
                        },
                    },
                    "required": ["path", "content"],
                }
            }
        }
    ]
    resp = client.chat.completions.create(
        model="gpt-5.3-codex",
        messages=[
            {"role": "user", "content": "Create a file at /tmp/hello.txt with content 'hello world'"}
        ],
        tools=verbose_tools,
        tool_choice="required",
        stream=False,
    )
    assert_eq("finish_reason", resp.choices[0].finish_reason, "tool_calls")
    tc = resp.choices[0].message.tool_calls[0]
    assert_eq("tool name", tc.function.name, "create_file")
    args = json.loads(tc.function.arguments)
    assert_isinstance("args is dict", args, dict)
    assert_truthy("path arg present", "path" in args)
    assert_truthy("content arg present", "content" in args)
    info(f"create_file args: {args}")


# ---------------------------------------------------------------------------
# Auto-detect URL from runtime.json
# ---------------------------------------------------------------------------
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
                    # Use 127.0.0.1 instead of the LAN IP for local testing
                    import re
                    url = re.sub(r"http://[\d.]+:", "http://127.0.0.1:", url)
                    return url.rstrip("/")
            except Exception:
                pass
    return "http://127.0.0.1:51824"


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------
def main():
    parser = argparse.ArgumentParser(description="LLM Hotspot integration tests")
    parser.add_argument("--url",     default=None, help="Base URL (default: auto-detect from runtime.json)")
    parser.add_argument("--api-key", default=os.environ.get("LLMHOTSPOT_API_KEY", ""), help="API key (hs_...)")
    parser.add_argument("--filter",  default="", help="Only run tests whose name contains this string")
    args = parser.parse_args()

    base_url = (args.url or detect_url()).rstrip("/")
    api_key  = args.api_key

    if not api_key:
        print(f"{RED}ERROR:{RESET} No API key provided.")
        print("  Pass --api-key hs_xxx  or  set LLMHOTSPOT_API_KEY=hs_xxx")
        sys.exit(1)

    print(f"\n{BOLD}LLM Hotspot Integration Tests{RESET}")
    print(f"  URL:     {base_url}")
    print(f"  Key:     {api_key[:12]}{'*' * (len(api_key) - 12) if len(api_key) > 12 else ''}")
    if args.filter:
        print(f"  Filter:  {args.filter!r}")

    client = OpenAI(api_key=api_key, base_url=f"{base_url}/v1")

    # --- Connectivity check ---
    print(f"\n{BOLD}Checking connectivity...{RESET}")
    import urllib.request, urllib.error
    try:
        urllib.request.urlopen(f"{base_url}/v1/models", timeout=5)
    except (urllib.error.URLError, OSError) as e:
        print(f"{RED}Cannot reach {base_url}: {e}{RESET}")
        print("Is LLM Hotspot running?")
        sys.exit(1)
    ok(f"Reachable at {base_url}")

    # --- Define test suite ---
    all_tests = [
        ("Models list",                          lambda: test_models_list(client)),
        ("Basic chat (non-streaming)",           lambda: test_basic_chat_non_streaming(client)),
        ("Streaming chat",                       lambda: test_streaming_chat(client)),
        ("Tool call – no tool_choice (→required)", lambda: test_tool_call_no_tool_choice(client)),
        ("Tool call – explicit required",        lambda: test_tool_call_explicit_required(client)),
        ("Tool call – explicit none",            lambda: test_tool_call_explicit_none(client)),
        ("Tool call – specific function",        lambda: test_tool_call_specific_function(client)),
        ("Streaming tool calls",                 lambda: test_streaming_tool_calls(client)),
        ("Multi-turn tool loop",                 lambda: test_multi_turn_tool_loop(client)),
        ("reasoning_effort (Chat style)",        lambda: test_reasoning_effort_passthrough(client)),
        ("reasoning.effort (Responses style)",   lambda: test_reasoning_effort_responses_style(client)),
        ("Compact schema → tool call intact",    lambda: test_compact_schema_does_not_break_tool_calls(client)),
        ("Auth rejection – missing token",       lambda: test_auth_rejection_missing_token(base_url)),
        ("Auth rejection – bad token",           lambda: test_auth_rejection_bad_token(base_url)),
        ("Invalid model rejected",               lambda: test_invalid_model_rejected(client)),
    ]

    if args.filter:
        tests = [(n, f) for n, f in all_tests if args.filter.lower() in n.lower()]
        if not tests:
            print(f"{YELLOW}No tests matched filter {args.filter!r}{RESET}")
            sys.exit(1)
    else:
        tests = all_tests

    start = time.time()
    for name, fn in tests:
        run_test(name, fn)

    # --- Summary ---
    elapsed = time.time() - start
    passed  = sum(1 for _, ok_, _ in RESULTS if ok_)
    failed  = sum(1 for _, ok_, _ in RESULTS if not ok_)

    print(f"\n{'─' * 56}")
    print(f"{BOLD}Results:{RESET}  {GREEN}{passed} passed{RESET}  {RED}{failed} failed{RESET}  ({elapsed:.1f}s)")
    print('─' * 56)

    if failed:
        print(f"\n{RED}Failed tests:{RESET}")
        for name, ok_, err in RESULTS:
            if not ok_:
                print(f"  {RED}✗ {name}{RESET}")
                first_line = err.split("\n")[0] if err else ""
                if first_line:
                    print(f"    {first_line}")
        sys.exit(1)
    else:
        print(f"\n{GREEN}{BOLD}All tests passed.{RESET}")


if __name__ == "__main__":
    main()
