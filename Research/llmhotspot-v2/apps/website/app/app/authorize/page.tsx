import { redirect } from "next/navigation";
import { z } from "zod";
import { brokerJson } from "@/lib/broker";
import { getCustomerSession } from "@/lib/sessions";

const schema = z.object({
  state: z.string().min(1),
  code_challenge: z.string().min(8),
  device_id: z.string().min(1),
  redirect_uri: z.string().url().default("llmhotspot://auth/callback")
});

type Props = {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
};

function toSingle(v: string | string[] | undefined): string {
  if (Array.isArray(v)) {
    return v[0] ?? "";
  }
  return v ?? "";
}

export default async function AppAuthorizePage({ searchParams }: Props) {
  const params = await searchParams;
  const parsed = schema.safeParse({
    state: toSingle(params.state),
    code_challenge: toSingle(params.code_challenge),
    device_id: toSingle(params.device_id),
    redirect_uri: toSingle(params.redirect_uri) || "llmhotspot://auth/callback"
  });

  if (!parsed.success) {
    return (
      <main className="shell">
        <h1 className="text-3xl font-black">Invalid Authorization Request</h1>
        <p className="mt-3 text-slate-600">Missing or invalid app authorization parameters.</p>
      </main>
    );
  }

  const customer = await getCustomerSession();
  if (!customer) {
    return (
      <main className="shell">
        <h1 className="text-3xl font-black">Sign in to Continue</h1>
        <p className="mt-3 text-slate-600">
          Open your customer access link first, then return here to finish app sign-in.
        </p>
        <a className="btn btn-primary mt-4 inline-flex" href="/manage">
          Open Customer Management
        </a>
      </main>
    );
  }

  let out: any;
  try {
    out = await brokerJson("/api/app/oauth/authorize", {
      method: "POST",
      body: {
        email: customer.email,
        state: parsed.data.state,
        code_challenge: parsed.data.code_challenge,
        device_id: parsed.data.device_id
      }
    });
  } catch (error: any) {
    const msg = error?.message ?? "authorization_failed";
    if (msg === "no_active_subscription") {
      return (
        <main className="shell">
          <h1 className="text-3xl font-black">Upgrade Required</h1>
          <p className="mt-3 text-slate-600">
            Your account does not have an active paid subscription yet. Complete checkout, then try app sign-in again.
          </p>
          <a className="btn btn-primary mt-4 inline-flex" href="/#pricing">
            Open Checkout
          </a>
        </main>
      );
    }
    return (
      <main className="shell">
        <h1 className="text-3xl font-black">Authorization Failed</h1>
        <p className="mt-3 text-slate-600">{msg}</p>
      </main>
    );
  }

  const code = String(out?.code ?? "");
  const state = String(out?.state ?? parsed.data.state);
  const callback = new URL(parsed.data.redirect_uri);
  callback.searchParams.set("code", code);
  callback.searchParams.set("state", state);
  redirect(callback.toString());
}
