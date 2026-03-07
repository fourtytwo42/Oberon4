import { NextRequest } from "next/server";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { getAdminSession } from "@/lib/sessions";

export async function GET(req: NextRequest) {
  const session = await getAdminSession();
  if (!session) return fail("unauthorized", 401);
  const qs = req.nextUrl.search || "";
  try {
    const data = await brokerJson(`/admin/customer-tree${qs}`, {
      method: "GET",
      headers: { "x-admin-session": session.brokerSessionId }
    });
    return ok(data);
  } catch (error: any) {
    return fail(error?.message ?? "customer_tree_failed", error?.status ?? 502);
  }
}
