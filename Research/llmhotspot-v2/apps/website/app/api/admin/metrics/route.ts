import { NextRequest } from "next/server";
import { cacheGet, cacheSet } from "@/lib/redis";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { getAdminSession } from "@/lib/sessions";

async function requireAdminBrokerSession() {
  const session = await getAdminSession();
  if (!session) {
    throw new Error("unauthorized");
  }
  return session.brokerSessionId;
}

export async function GET(_req: NextRequest) {
  let brokerSessionId: string;
  try {
    brokerSessionId = await requireAdminBrokerSession();
  } catch {
    return fail("unauthorized", 401);
  }

  const cacheKey = "admin:metrics:v1";
  const cached = await cacheGet(cacheKey);
  if (cached) {
    return ok(JSON.parse(cached));
  }

  try {
    const data = await brokerJson("/admin/metrics", {
      method: "GET",
      headers: { "x-admin-session": brokerSessionId }
    });
    await cacheSet(cacheKey, JSON.stringify(data), 20);
    return ok(data);
  } catch (error: any) {
    return fail(error?.message ?? "metrics_failed", error?.status ?? 502);
  }
}
