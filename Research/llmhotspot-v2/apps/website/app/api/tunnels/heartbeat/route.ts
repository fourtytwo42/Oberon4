import { NextRequest } from "next/server";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";

export async function POST(req: NextRequest) {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return fail("invalid_json", 400);
  }
  try {
    const data = await brokerJson("/api/tunnels/heartbeat", { method: "POST", body });
    return ok(data);
  } catch (error: any) {
    return fail(error?.message ?? "tunnel_heartbeat_failed", error?.status ?? 502);
  }
}
