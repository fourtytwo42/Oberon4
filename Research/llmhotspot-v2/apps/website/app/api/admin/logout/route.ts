import { NextRequest } from "next/server";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";
import { clearAdminCookie, clearAdminSession, getAdminSession } from "@/lib/sessions";

export async function POST(_req: NextRequest) {
  const session = await getAdminSession();
  if (session) {
    try {
      await brokerJson("/admin/auth/logout", {
        method: "POST",
        headers: { "x-admin-session": session.brokerSessionId },
        body: {}
      });
    } catch {
      // best effort
    }
    await clearAdminSession(session.sessionId);
  }
  await clearAdminCookie();
  return ok({ loggedOut: true });
}

export async function GET() {
  return fail("method_not_allowed", 405);
}
