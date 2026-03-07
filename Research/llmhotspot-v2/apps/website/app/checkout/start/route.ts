import { NextRequest, NextResponse } from "next/server";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { getEnv } from "@/lib/env";
import { MAX_SELF_SERVE_SEATS } from "@/lib/pricing";
import { issueOneTimeAccessToken } from "@/lib/customer_access";
import { sendCustomerAccessEmail } from "@/lib/smtp";

const schema = z.object({
  email: z.string().email(),
  seats: z.coerce.number().int().min(1).max(MAX_SELF_SERVE_SEATS)
});

function redirectError(req: NextRequest, code: string): NextResponse {
  const url = new URL("/", req.url);
  url.hash = "pricing";
  url.searchParams.set("checkout_error", code);
  return NextResponse.redirect(url, 303);
}

export async function POST(req: NextRequest) {
  const form = await req.formData();
  const parsed = schema.safeParse({
    email: String(form.get("email") ?? ""),
    seats: String(form.get("seats") ?? "1")
  });
  if (!parsed.success) {
    return redirectError(req, "invalid_request");
  }

  const env = getEnv();
  let brokerResp: any;
  try {
    brokerResp = await brokerJson("/billing/create-checkout", {
      method: "POST",
      body: {
        email: parsed.data.email.toLowerCase(),
        seats: parsed.data.seats,
        success_url: env.WEBSITE_CHECKOUT_SUCCESS_URL,
        cancel_url: env.WEBSITE_CHECKOUT_CANCEL_URL
      }
    });
  } catch (error: any) {
    return redirectError(req, error?.message ?? "checkout_failed");
  }

  // Best effort: do not block checkout redirect on email issues.
  try {
    const accessToken = await issueOneTimeAccessToken(parsed.data.email.toLowerCase());
    await sendCustomerAccessEmail(
      parsed.data.email.toLowerCase(),
      accessToken,
      "Your LLM Hotspot management link"
    );
  } catch {
    // ignore email send failures on checkout start
  }

  const target = String(brokerResp?.checkoutUrl ?? "");
  if (!target) {
    return redirectError(req, "missing_checkout_url");
  }
  return NextResponse.redirect(target, 303);
}
