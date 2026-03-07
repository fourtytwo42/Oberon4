import { NextRequest } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { issueOneTimeAccessToken } from "@/lib/customer_access";
import { fail, ok } from "@/lib/http";
import { incrementRateLimit } from "@/lib/redis";
import { sendCustomerAccessEmail } from "@/lib/smtp";

const schema = z.object({
  email: z.string().email()
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

  const email = parsed.data.email.toLowerCase();
  const rl = await incrementRateLimit(`customer:access:req:${email}`, 10 * 60);
  if (rl > 8) {
    return fail("rate_limited", 429);
  }

  let customerExists = false;
  try {
    await brokerJson("/api/customer/context", {
      method: "POST",
      body: { email, challenge_id: "", code: "" }
    });
    customerExists = true;
  } catch {
    customerExists = false;
  }

  if (customerExists) {
    try {
      const accessToken = await issueOneTimeAccessToken(email);
      await sendCustomerAccessEmail(email, accessToken, "Your LLM Hotspot management link");
    } catch {
      // Do not leak details of account existence or SMTP status.
    }
  }

  return ok({
    submitted: true,
    message: "If your email is on an active account, a management link has been sent."
  });
}
