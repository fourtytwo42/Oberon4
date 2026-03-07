import { NextRequest } from "next/server";
import { z } from "zod";
import { getEnv } from "@/lib/env";
import { fail, ok } from "@/lib/http";
import { sendBillingReminderEmail } from "@/lib/smtp";

const reminderSchema = z.object({
  reminder_id: z.string().min(1),
  email: z.string().email(),
  subscription_id: z.string().min(1),
  license_id: z.string().nullable().optional(),
  reason: z.string().min(1),
  due_unix: z.number().int().nonnegative(),
  grace_expires_unix: z.number().int().nonnegative().nullable().optional(),
  current_period_end_unix: z.number().int().nonnegative().nullable().optional(),
  seats: z.number().int().positive().default(1),
  status: z.string().default("unknown")
});

const requestSchema = z.object({
  reminders: z.array(reminderSchema).max(100)
});

export async function POST(req: NextRequest) {
  const env = getEnv();
  const expected = env.WEBSITE_REMINDER_INTERNAL_SECRET;
  if (!expected) {
    return fail("reminder_internal_secret_not_configured", 501);
  }
  const provided = req.headers.get("x-internal-reminder-secret") ?? "";
  if (provided !== expected) {
    return fail("unauthorized", 401);
  }

  let bodyRaw: unknown;
  try {
    bodyRaw = await req.json();
  } catch {
    return fail("invalid_json", 400);
  }
  const parsed = requestSchema.safeParse(bodyRaw);
  if (!parsed.success) {
    return fail("invalid_request", 400, { issues: parsed.error.issues });
  }

  const results: Array<{ reminder_id: string; ok: boolean; error?: string }> = [];
  for (const reminder of parsed.data.reminders) {
    try {
      await sendBillingReminderEmail({
        reminderId: reminder.reminder_id,
        email: reminder.email,
        subscriptionId: reminder.subscription_id,
        licenseId: reminder.license_id ?? null,
        reason: reminder.reason,
        dueUnix: reminder.due_unix,
        graceExpiresUnix: reminder.grace_expires_unix ?? null,
        currentPeriodEndUnix: reminder.current_period_end_unix ?? null,
        seats: reminder.seats,
        status: reminder.status
      });
      results.push({ reminder_id: reminder.reminder_id, ok: true });
    } catch (error) {
      const message = error instanceof Error ? error.message : "send_failed";
      results.push({ reminder_id: reminder.reminder_id, ok: false, error: message });
    }
  }

  return ok({ results });
}
