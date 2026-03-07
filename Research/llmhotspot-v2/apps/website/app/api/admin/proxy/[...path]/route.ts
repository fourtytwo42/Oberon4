import { NextRequest } from "next/server";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { getAdminSession } from "@/lib/sessions";

export async function POST(
  req: NextRequest,
  { params }: { params: Promise<{ path: string[] }> }
) {
  const session = await getAdminSession();
  if (!session) {
    return fail("unauthorized", 401);
  }
  const resolved = await params;
  const path = resolved.path.join("/");
  let body: unknown = {};
  try {
    body = await req.json();
  } catch {
    // keep empty body for non-json posts
  }
  try {
    const data = await brokerJson(`/admin/${path}`, {
      method: "POST",
      headers: { "x-admin-session": session.brokerSessionId },
      body
    });
    return ok(data);
  } catch (error: any) {
    return fail(error?.message ?? "admin_proxy_failed", error?.status ?? 502);
  }
}
