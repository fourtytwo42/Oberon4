import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { getCustomerSession, updateCustomerSessionContext } from "@/lib/sessions";

const schema = z.object({ bindingId: z.string().min(1) });

export async function POST(req: NextRequest) {
  const session = await getCustomerSession();
  if (!session) {
    return fail("unauthorized", 401);
  }

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

  try {
    await brokerJson("/api/customer/device/retire", {
      method: "POST",
      body: { email: session.email, binding_id: parsed.data.bindingId }
    });
    const refreshed = await brokerJson("/api/customer/context", {
      method: "POST",
      body: { email: session.email, challenge_id: "", code: "" }
    });
    await updateCustomerSessionContext(session.sessionId, JSON.stringify(refreshed));
    return ok({ retired: true, context: refreshed });
  } catch (error: any) {
    return fail(error?.message ?? "retire_failed", error?.status ?? 502);
  }
}
