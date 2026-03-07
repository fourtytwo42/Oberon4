import { NextRequest } from "next/server";
import { brokerJson } from "@/lib/broker";
import { getEnv } from "@/lib/env";
import { fail, ok } from "@/lib/http";
import { getCustomerSession } from "@/lib/sessions";

export async function POST(_req: NextRequest) {
  const session = await getCustomerSession();
  if (!session) {
    return fail("unauthorized", 401);
  }
  try {
    const env = getEnv();
    const data = await brokerJson("/api/customer/portal-session", {
      method: "POST",
      body: {
        email: session.email,
        challenge_id: "",
        code: "",
        return_url: env.WEBSITE_CHECKOUT_CANCEL_URL
      }
    });
    return ok({ portalUrl: data.portalUrl });
  } catch (error: any) {
    return fail(error?.message ?? "portal_failed", error?.status ?? 502);
  }
}
