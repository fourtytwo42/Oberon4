import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import {
  createCustomerSession,
  setCustomerCookie,
  updateCustomerSessionContext
} from "@/lib/sessions";

const schema = z.object({
  email: z.string().email(),
  challengeId: z.string().min(1),
  code: z.string().min(4)
});

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
  const challengeId = parsed.data.challengeId.trim();
  const code = parsed.data.code.trim();

  let context: any;
  try {
    context = await brokerJson("/api/customer/context", {
      method: "POST",
      body: { email, challenge_id: challengeId, code }
    });
  } catch (error: any) {
    return fail(error?.message ?? "otp_verify_failed", error?.status ?? 401);
  }

  const sessionId = await createCustomerSession(email, JSON.stringify(context));
  await setCustomerCookie(sessionId);
  await updateCustomerSessionContext(sessionId, JSON.stringify(context));
  return ok({ authenticated: true, customer: context.customer });
}
