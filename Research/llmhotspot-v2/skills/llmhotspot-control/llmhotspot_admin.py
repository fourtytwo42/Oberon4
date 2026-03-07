#!/usr/bin/env python3
"""LLM Hotspot admin helper for OpenClaw skills.

Supports local API bases and remote console URLs with embedded admin_key.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from typing import Any, Dict, Optional, Tuple
from urllib.parse import parse_qs, urlparse

import requests


@dataclass
class Target:
    base_v1: str
    admin_key: Optional[str]


def _normalize_base(raw: str) -> Tuple[str, Optional[str]]:
    raw = (raw or "").strip()
    if not raw:
        raise ValueError("empty target")

    parsed = urlparse(raw)
    if not parsed.scheme or not parsed.netloc:
        raise ValueError(f"target must be absolute URL: {raw}")

    admin_key = parse_qs(parsed.query).get("admin_key", [None])[0]
    path = parsed.path.rstrip("/")

    if path.endswith("/remote"):
        base_path = "/v1"
    elif path.endswith("/v1"):
        base_path = "/v1"
    elif path == "":
        base_path = "/v1"
    else:
        base_path = path

    return f"{parsed.scheme}://{parsed.netloc}{base_path}", admin_key


def resolve_target(raw_target: Optional[str], explicit_admin_key: Optional[str]) -> Target:
    candidate = (
        raw_target
        or os.getenv("LLMHOTSPOT_TARGET")
        or os.getenv("LLMHOTSPOT_REMOTE_CONSOLE_URL")
        or "http://127.0.0.1:11434/v1"
    )
    base_v1, parsed_key = _normalize_base(candidate)
    admin_key = explicit_admin_key or os.getenv("LLMHOTSPOT_ADMIN_KEY") or parsed_key
    return Target(base_v1=base_v1, admin_key=admin_key)


def call_api(target: Target, method: str, path: str, body: Optional[Dict[str, Any]]) -> requests.Response:
    clean_path = path if path.startswith("/") else f"/{path}"
    url = f"{target.base_v1}{clean_path}"

    headers: Dict[str, str] = {"content-type": "application/json"}
    if target.admin_key:
        headers["authorization"] = f"Bearer {target.admin_key}"

    return requests.request(method=method.upper(), url=url, headers=headers, json=body, timeout=30)


def print_response(resp: requests.Response) -> int:
    try:
        payload = resp.json()
    except Exception:
        payload = {"raw": (resp.text or "")[:2000]}

    out = {
        "status": resp.status_code,
        "ok": resp.ok,
        "body": payload,
    }
    print(json.dumps(out, indent=2, sort_keys=True))
    return 0 if resp.ok else 1


def parse_body(raw: Optional[str]) -> Optional[Dict[str, Any]]:
    if raw is None:
        return None
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ValueError(f"invalid JSON body: {exc}") from exc
    if parsed is None:
        return None
    if not isinstance(parsed, dict):
        raise ValueError("JSON body must be an object")
    return parsed


def cmd_discover(target: Target) -> int:
    paths = [
        "/auth/status",
        "/endpoints",
        "/models",
        "/oauth/accounts",
        "/diagnostics",
    ]
    output = {"base_v1": target.base_v1, "calls": []}
    code = 0
    for p in paths:
        resp = call_api(target, "GET", p, None)
        try:
            body = resp.json()
        except Exception:
            body = {"raw": (resp.text or "")[:1000]}
        output["calls"].append({"path": p, "status": resp.status_code, "ok": resp.ok, "body": body})
        if not resp.ok:
            code = 1
    print(json.dumps(output, indent=2, sort_keys=True))
    return code


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="LLM Hotspot admin helper")
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--target", help="Remote console URL or API base URL")
    common.add_argument("--admin-key", help="Admin key (Bearer token)")

    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("discover", help="Get a quick snapshot of app state", parents=[common])
    sub.add_parser("auth-status", help="GET /v1/auth/status", parents=[common])
    sub.add_parser("endpoints", help="GET /v1/endpoints", parents=[common])
    sub.add_parser("oauth-accounts", help="GET /v1/oauth/accounts", parents=[common])
    sub.add_parser("diagnostics", help="GET /v1/diagnostics", parents=[common])

    tunnel = sub.add_parser("set-tunnel-mode", help="POST /v1/tunnel-mode", parents=[common])
    tunnel.add_argument("--mode", choices=["quick", "managed", "off"], required=True)

    req = sub.add_parser("request", help="Generic request", parents=[common])
    req.add_argument("--method", required=True)
    req.add_argument("--path", required=True, help="Path after /v1, e.g. /keys or /v1/keys")
    req.add_argument("--body", help="JSON object string")

    args = parser.parse_args(argv)

    try:
        target = resolve_target(args.target, args.admin_key)
    except Exception as exc:
        print(json.dumps({"error": str(exc)}))
        return 2

    if args.command == "discover":
        return cmd_discover(target)
    if args.command == "auth-status":
        return print_response(call_api(target, "GET", "/auth/status", None))
    if args.command == "endpoints":
        return print_response(call_api(target, "GET", "/endpoints", None))
    if args.command == "oauth-accounts":
        return print_response(call_api(target, "GET", "/oauth/accounts", None))
    if args.command == "diagnostics":
        return print_response(call_api(target, "GET", "/diagnostics", None))
    if args.command == "set-tunnel-mode":
        return print_response(call_api(target, "POST", "/tunnel-mode", {"mode": args.mode}))
    if args.command == "request":
        path = args.path
        if path.startswith("/v1/"):
            path = path[3:]
        body = parse_body(args.body)
        return print_response(call_api(target, args.method, path, body))

    print(json.dumps({"error": f"unsupported command: {args.command}"}))
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
