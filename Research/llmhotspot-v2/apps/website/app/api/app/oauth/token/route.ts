import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { fail, ok } from "@/lib/http";

const schema = z.object({
  code: z.string().min(1),
  code_verifier: z.string().min(1),
  device_id: z.string().min(1),
  device_name: z.string().trim().min(1).max(128).optional(),
  os: z.string().trim().min(1).max(64).optional()
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
    const forwardedHeaders: Record<string, string> = {};
    const country = req.headers.get("cf-ipcountry")?.trim().toUpperCase() ?? "";
    if (/^[A-Z]{2}$/.test(country)) {
      forwardedHeaders["cf-ipcountry"] = country;
    }
    const cfIp = req.headers.get("cf-connecting-ip")?.trim() ?? "";
    if (cfIp) {
      forwardedHeaders["x-real-ip"] = cfIp;
    }
    const payload = await brokerJson("/api/app/oauth/token", {
      method: "POST",
      headers: forwardedHeaders,
      body: parsed.data
    });
    return ok(payload);
  } catch (error: any) {
    return fail(error?.message ?? "oauth_token_failed", error?.status ?? 502, error?.payload);
  }
}
