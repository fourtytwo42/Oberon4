import { NextRequest } from "next/server";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { getCustomerSession, updateCustomerSessionContext } from "@/lib/sessions";

export async function GET(_req: NextRequest) {
  const session = await getCustomerSession();
  if (!session) {
    return fail("unauthorized", 401);
  }
  try {
    const live = await brokerJson("/api/customer/context", {
      method: "POST",
      body: { email: session.email, challenge_id: "", code: "" }
    });
    await updateCustomerSessionContext(session.sessionId, JSON.stringify(live));
    return ok(live);
  } catch (error: any) {
    if (!session.contextJson) {
      return fail(error?.message ?? "customer_context_unavailable", error?.status ?? 409);
    }
    return ok(JSON.parse(session.contextJson));
  }
}
