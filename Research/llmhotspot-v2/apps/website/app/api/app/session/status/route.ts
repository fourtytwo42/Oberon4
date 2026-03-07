import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";

const schema = z.object({
  session_token: z.string().min(1),
  device_id: z.string().min(1)
});

export async function POST(req: NextRequest) {
  let bodyRaw: unknown;
  try {
    bodyRaw = await req.json();
  } catch {
    return fail("invalid_json", 400);
  }
  const parsed = schema.safeParse(bodyRaw);
  if (!parsed.success) {
    return fail("invalid_request", 400, { issues: parsed.error.issues });
  }
  try {
    const payload = await brokerJson("/api/app/session/status", {
      method: "POST",
      body: parsed.data
    });
    return ok(payload);
  } catch (error: any) {
    return fail(error?.message ?? "session_status_failed", error?.status ?? 502, error?.payload);
  }
}

