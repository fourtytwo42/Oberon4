import { NextRequest } from "next/server";
import { brokerRequest } from "@/lib/broker";
import { fail } from "@/lib/http";
import { getAdminSession } from "@/lib/sessions";

export async function GET(req: NextRequest) {
  const session = await getAdminSession();
  if (!session) return fail("unauthorized", 401);
  const qs = req.nextUrl.search || "";
  const upstream = await brokerRequest(`/admin/export/subscriptions.csv${qs}`, {
    method: "GET",
    noJson: true,
    headers: { "x-admin-session": session.brokerSessionId, accept: "text/csv" }
  });
  const body = await upstream.text();
  if (!upstream.ok) return fail(body || "export_failed", upstream.status);
  return new Response(body, { status: 200, headers: { "content-type": "text/csv; charset=utf-8" } });
}
