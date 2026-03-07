import { NextRequest } from "next/server";
import { z } from "zod";
import { getAdminSession } from "@/lib/sessions";
import { fail, ok } from "@/lib/http";
import { getSmtpSettings, upsertSmtpSettings } from "@/lib/smtp";

const schema = z.object({
  host: z.string().min(1),
  port: z.number().int().min(1).max(65535),
  secure: z.boolean(),
  username: z.string().min(1),
  password: z.string().optional(),
  fromEmail: z.string().email(),
  fromName: z.string().optional()
});

export async function GET() {
  const session = await getAdminSession();
  if (!session) {
    return fail("admin_auth_required", 401);
  }
  const settings = await getSmtpSettings();
  return ok({
    settings: settings
      ? {
          host: settings.host,
          port: settings.port,
          secure: settings.secure,
          username: settings.username,
          fromEmail: settings.fromEmail,
          fromName: settings.fromName,
          passwordSet: settings.password.length > 0
        }
      : null
  });
}

export async function POST(req: NextRequest) {
  const session = await getAdminSession();
  if (!session) {
    return fail("admin_auth_required", 401);
  }

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

  const current = await getSmtpSettings();
  const password = parsed.data.password?.trim() || current?.password || "";
  if (!password) {
    return fail("smtp_password_required", 400);
  }

  await upsertSmtpSettings({
    host: parsed.data.host.trim(),
    port: parsed.data.port,
    secure: parsed.data.secure,
    username: parsed.data.username.trim(),
    password,
    fromEmail: parsed.data.fromEmail.trim().toLowerCase(),
    fromName: parsed.data.fromName?.trim() || null
  });

  return ok({ saved: true });
}
