import { NextRequest } from "next/server";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { getAdminSession } from "@/lib/sessions";

export async function POST(_req: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const session = await getAdminSession();
  if (!session) return fail("unauthorized", 401);
  const { id } = await params;
  try {
    const data = await brokerJson(`/admin/subscriptions/${encodeURIComponent(id)}/unsuspend`, {
      method: "POST",
      headers: { "x-admin-session": session.brokerSessionId },
      body: {}
    });
    return ok(data);
  } catch (error: any) {
    return fail(error?.message ?? "unsuspend_failed", error?.status ?? 502);
  }
}
