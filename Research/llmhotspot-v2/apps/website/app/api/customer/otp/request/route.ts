import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { incrementRateLimit } from "@/lib/redis";

const schema = z.object({ email: z.string().email() });

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

  const email = parsed.data.email.toLowerCase();
  const rl = await incrementRateLimit(`otp:req:${email}`, 10 * 60);
  if (rl > 6) {
    return fail("rate_limited", 429);
  }

  let brokerResp: any;
  try {
    brokerResp = await brokerJson("/api/otp/request", {
      method: "POST",
      body: { email, purpose: "management" }
    });
  } catch (error: any) {
    return fail(error?.message ?? "otp_request_failed", error?.status ?? 502);
  }
  return ok({ challengeId: brokerResp.challengeId, expiresUnix: brokerResp.expiresUnix });
}
