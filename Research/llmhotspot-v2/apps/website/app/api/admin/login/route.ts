import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { createAdminSession, setAdminCookie } from "@/lib/sessions";
import { incrementRateLimit } from "@/lib/redis";

const schema = z.object({ adminSecret: z.string().min(1) });

export async function POST(req: NextRequest) {
  let bodyRaw: unknown;
  try {
    bodyRaw = await req.json();
  } catch {
    return fail("invalid_json", 400);
  }
  const parsed = schema.safeParse(bodyRaw);
  if (!parsed.success) {
    return fail("invalid_request", 400);
  }

  const ip = req.headers.get("x-forwarded-for")?.split(",")[0]?.trim() ?? "unknown";
  const rl = await incrementRateLimit(`admin:login:${ip}`, 10 * 60);
  if (rl > 25) {
    return fail("rate_limited", 429);
  }

  let brokerResp: any;
  try {
    brokerResp = await brokerJson("/admin/auth/login", {
      method: "POST",
      body: { admin_secret: parsed.data.adminSecret }
    });
  } catch (error: any) {
    return fail(error?.message ?? "login_failed", error?.status ?? 401);
  }

  const sessionId = await createAdminSession(brokerResp.sessionId);
  await setAdminCookie(sessionId);
  return ok({ authenticated: true, expiresUnix: brokerResp.expiresUnix });
}
