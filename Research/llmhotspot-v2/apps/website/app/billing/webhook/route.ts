import { NextRequest, NextResponse } from "next/server";
import { getEnv } from "@/lib/env";

export async function POST(req: NextRequest) {
  const env = getEnv();
  const brokerBase = env.WEBSITE_BROKER_BASE_URL.replace(/\/$/, "");
  const body = await req.text();
  const stripeSignature = req.headers.get("stripe-signature") ?? "";
  const contentType = req.headers.get("content-type") ?? "application/json";

  let upstream: Response;
  try {
    upstream = await fetch(`${brokerBase}/billing/webhook`, {
      method: "POST",
      headers: {
        "content-type": contentType,
        "stripe-signature": stripeSignature
      },
      body
    });
  } catch (error: any) {
    return NextResponse.json(
      { ok: false, error: "webhook_forward_failed", message: error?.message ?? "unknown_error" },
      { status: 502 }
    );
  }

  const responseText = await upstream.text();
  const ct = upstream.headers.get("content-type") ?? "application/json";
  return new NextResponse(responseText, {
    status: upstream.status,
    headers: { "content-type": ct }
  });
}

export async function GET() {
  return NextResponse.json({ ok: false, error: "method_not_allowed" }, { status: 405 });
}
