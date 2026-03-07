#!/usr/bin/env python3
"""
LLM Hotspot integration tests using the Google Gemini client library.

Usage:
  python test_hotspot_gemini.py --api-key hs_xxx
  LLMHOTSPOT_API_KEY=hs_xxx python test_hotspot_gemini.py
  python test_hotspot_gemini.py --url http://127.0.0.1:51824
"""

import argparse
import json
import os
import sys
import time
import traceback
from pathlib import Path

try:
    from google import genai
    from google.genai import types
except ImportError:
    print("ERROR: google-genai package not installed. Run: pip install google-genai")
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


def make_client(base_url: str, api_key: str):
    # We target LLM Hotspot's Gemini-compatible route:
    #   /v1/google/models/{model}:generateContent
    # The google-genai SDK will emit "models/{model}:generateContent";
    # base_url is therefore pinned to ".../v1/google" and api_version=None.
    return genai.Client(
        api_key="llmhotspot-local-test",
        http_options=types.HttpOptions(
            base_url=f"{base_url}/v1/google",
            # Empty string forces no injected "/v1beta" prefix for custom base URLs.
            api_version="",
            headers={"Authorization": f"Bearer {api_key}"},
            timeout=60_000,
        ),
    )


def response_text(resp) -> str:
    txt = getattr(resp, "text", None)
    if isinstance(txt, str) and txt.strip():
        return txt.strip()
    parts = []
    for cand in getattr(resp, "candidates", []) or []:
        content = getattr(cand, "content", None)
        if not content:
            continue
        for part in getattr(content, "parts", []) or []:
            t = getattr(part, "text", None)
            if t:
                parts.append(t)
    return "\n".join(parts).strip()


def test_basic_generate_content(client):
    resp = client.models.generate_content(
        model="gemini-2.5-flash",
        contents="Reply with exactly: HOTSPOT_GEMINI_OK",
    )
    txt = response_text(resp)
    info(f"text={txt!r}")
    assert txt, "expected non-empty text"
    assert "HOTSPOT_GEMINI_OK" in txt, f"expected marker in text, got {txt!r}"
    ok("basic generate_content returned expected text")


def test_stream_generate_content(client):
    stream = client.models.generate_content_stream(
        model="gemini-2.5-flash",
        contents="Reply with exactly: HOTSPOT_GEMINI_STREAM_OK",
    )
    chunks = list(stream)
    assert chunks, "expected at least one stream chunk"
    text = "".join(getattr(c, "text", "") or "" for c in chunks).strip()
    info(f"stream chunks={len(chunks)} text={text!r}")
    assert text, "expected non-empty streamed text"
    assert "HOTSPOT_GEMINI_STREAM_OK" in text, f"expected marker in text, got {text!r}"
    ok("generate_content_stream returned expected text")


def test_invalid_model(client):
    try:
        client.models.generate_content(
            model="gemini-imaginary-not-real",
            contents="hello",
        )
        raise AssertionError("expected API error for invalid model")
    except Exception as e:
        code = getattr(e, "status_code", None)
        info(f"status={code!r} err={type(e).__name__}: {e}")
        if code is not None:
            assert int(code) in (400, 404, 502), f"unexpected status code: {code}"
        ok("invalid model rejected")


def main():
    parser = argparse.ArgumentParser(description="Gemini SDK integration tests for LLM Hotspot")
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

    print(f"\n{BOLD}Gemini SDK Compatibility Tests{RESET}")
    print(f"  URL: {base_url}")
    print(f"  Key: {api_key[:12]}{'*' * (len(api_key) - 12) if len(api_key) > 12 else ''}")

    client = make_client(base_url, api_key)
    tests = [
        ("Basic generate_content", lambda: test_basic_generate_content(client)),
        ("Streaming generate_content_stream", lambda: test_stream_generate_content(client)),
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
