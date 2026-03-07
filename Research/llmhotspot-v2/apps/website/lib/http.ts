import { NextResponse } from "next/server";

export function ok(data: unknown, init?: ResponseInit): NextResponse {
  return NextResponse.json({ ok: true, ...((data as object) ?? {}) }, init);
}

export function fail(error: string, status = 400, extra?: Record<string, unknown>): NextResponse {
  return NextResponse.json({ ok: false, error, ...(extra ?? {}) }, { status });
}
