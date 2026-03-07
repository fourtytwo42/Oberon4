import { NextRequest } from "next/server";
import { z } from "zod";
import { getAdminSession } from "@/lib/sessions";
import { fail, ok } from "@/lib/http";
import { getSmtpSettings, sendSmtpTestEmail, SmtpSettings } from "@/lib/smtp";

const schema = z.object({
  testEmail: z.string().email(),
  host: z.string().optional(),
  port: z.number().int().min(1).max(65535).optional(),
  secure: z.boolean().optional(),
  username: z.string().optional(),
  password: z.string().optional(),
  fromEmail: z.string().email().optional(),
  fromName: z.string().optional()
});

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
  const host = parsed.data.host?.trim() || current?.host || "";
  const port = parsed.data.port ?? current?.port ?? 0;
  const secure = parsed.data.secure ?? current?.secure ?? false;
  const username = parsed.data.username?.trim() || current?.username || "";
  const password = parsed.data.password?.trim() || current?.password || "";
  const fromEmail = parsed.data.fromEmail?.trim().toLowerCase() || current?.fromEmail || "";
  const fromName = parsed.data.fromName?.trim() || current?.fromName || null;

  if (!host || !port || !username || !password || !fromEmail) {
    return fail("smtp_incomplete", 400);
  }

  const settings: SmtpSettings = {
    host,
    port,
    secure,
    username,
    password,
    fromEmail,
    fromName
  };

  try {
    await sendSmtpTestEmail(settings, parsed.data.testEmail.trim().toLowerCase());
  } catch (error) {
    const err = error as {
      message?: string;
      code?: string;
      command?: string;
      response?: string;
      responseCode?: number;
    };
    const message = err?.message ?? "unknown_error";
    let hint: string | undefined;
    if (/Invalid login|Username and Password not accepted|EAUTH|535/i.test(message)) {
      hint =
        "SMTP auth failed. For Gmail, use an App Password (not your normal password) and ensure 2-Step Verification is enabled.";
    } else if (/ETIMEDOUT|ECONNREFUSED|ENOTFOUND/i.test(message)) {
      hint = "Connection failed. Check host/port, firewall rules, and whether TLS/SSL setting matches provider requirements.";
    }
    return fail("smtp_test_failed", 400, {
      message,
      code: err?.code,
      command: err?.command,
      responseCode: err?.responseCode,
      response: err?.response,
      hint
    });
  }

  return ok({ sent: true });
}
