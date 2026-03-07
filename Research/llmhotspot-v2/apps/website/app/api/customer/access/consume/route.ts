import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { consumeAccessToken } from "@/lib/customer_access";
import { fail, ok } from "@/lib/http";
import { createCustomerSession, setCustomerCookie, updateCustomerSessionContext } from "@/lib/sessions";

const schema = z.object({
  accessToken: z.string().min(10)
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
    return fail("invalid_request", 400, { issues: parsed.error.issues });
  }

  const accessToken = parsed.data.accessToken.trim();
  const email = await consumeAccessToken(accessToken);
  if (!email) {
    return fail("invalid_access_link", 401);
  }

  let context: any;
  try {
    context = await brokerJson("/api/customer/context", {
      method: "POST",
      body: { email, challenge_id: "", code: "" }
    });
  } catch (error: any) {
    if (error?.message === "customer_not_found") {
      context = {
        pending: true,
        reason: "customer_not_found",
        customer: { email },
        licenses: []
      };
    } else {
      return fail(error?.message ?? "customer_context_unavailable", error?.status ?? 502);
    }
  }

  const sessionId = await createCustomerSession(email, JSON.stringify(context));
  await setCustomerCookie(sessionId);
  await updateCustomerSessionContext(sessionId, JSON.stringify(context));
  return ok({ authenticated: true, customer: context.customer });
}
