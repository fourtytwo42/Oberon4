import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { ensureWebsiteSchema, getDb } from "@/lib/db";
import { getEnv } from "@/lib/env";
import { fail, ok } from "@/lib/http";
import { MAX_SELF_SERVE_SEATS } from "@/lib/pricing";
import { issueOneTimeAccessToken } from "@/lib/customer_access";
import { sendCustomerAccessEmail } from "@/lib/smtp";

const schema = z.object({
  email: z.string().email(),
  seats: z.number().int().min(1).max(MAX_SELF_SERVE_SEATS)
});

function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

export async function POST(req: NextRequest) {
  let bodyRaw: unknown;
  try {
    bodyRaw = await req.json();
  } catch {
    return fail("invalid_json", 400);
  }
  const parsed = schema.safeParse(bodyRaw);
  if (!parsed.success) {
    return fail("invalid_request", 400, { issues: parsed.error.issues });
  }

  const idempotencyKey = req.headers.get("x-idempotency-key")?.trim();
  await ensureWebsiteSchema();
  const db = getDb();
  const now = nowUnix();
  const routeKey = "checkout";

  if (idempotencyKey) {
    const existing = await db.query(
      `SELECT response_json FROM web_idempotency_keys WHERE idem_key = $1 AND route_key = $2 AND expires_unix >= $3 LIMIT 1`,
      [idempotencyKey, routeKey, now]
    );
    if (existing.rows.length > 0) {
      return ok(JSON.parse(existing.rows[0].response_json));
    }
  }

  const env = getEnv();
  let brokerResp: any;
  try {
    brokerResp = await brokerJson("/billing/create-checkout", {
      method: "POST",
      body: {
        email: parsed.data.email.toLowerCase(),
        seats: parsed.data.seats,
        success_url: env.WEBSITE_CHECKOUT_SUCCESS_URL,
        cancel_url: env.WEBSITE_CHECKOUT_CANCEL_URL
      }
    });
  } catch (error: any) {
    return fail(error?.message ?? "checkout_failed", error?.status ?? 502);
  }

  const responsePayload = {
    checkoutSessionId: brokerResp.checkoutSessionId,
    checkoutUrl: brokerResp.checkoutUrl,
    managementLinkSent: false
  };

  try {
    const accessToken = await issueOneTimeAccessToken(parsed.data.email.toLowerCase());
    await sendCustomerAccessEmail(
      parsed.data.email.toLowerCase(),
      accessToken,
      "Your LLM Hotspot management link"
    );
    responsePayload.managementLinkSent = true;
  } catch {
    responsePayload.managementLinkSent = false;
  }

  if (idempotencyKey) {
    await db.query(
      `INSERT INTO web_idempotency_keys(idem_key, route_key, response_json, created_unix, expires_unix)
       VALUES ($1, $2, $3, $4, $5)
       ON CONFLICT (idem_key) DO UPDATE SET response_json = EXCLUDED.response_json, expires_unix = EXCLUDED.expires_unix`,
      [idempotencyKey, routeKey, JSON.stringify(responsePayload), now, now + 60 * 60]
    );
  }

  return ok(responsePayload);
}
