"use client";

import { useEffect, useMemo, useState } from "react";
import { SiteHeader } from "@/components/site-header";
import { useSiteTheme } from "@/components/theme-provider";

type CustomerContext = any;

function formatUnix(unix?: number | null): string {
  if (!unix || !Number.isFinite(unix)) {
    return "-";
  }
  return new Date(unix * 1000).toLocaleString();
}

function formatDateOnly(unix?: number | null): string {
  if (!unix || !Number.isFinite(unix)) {
    return "-";
  }
  return new Date(unix * 1000).toLocaleDateString();
}

function shortId(value?: string | null): string {
  const raw = String(value ?? "").trim();
  if (!raw) {
    return "-";
  }
  if (raw.length <= 16) {
    return raw;
  }
  return `${raw.slice(0, 10)}...${raw.slice(-6)}`;
}

function statusPillClass(status?: string): string {
  const s = String(status ?? "").toLowerCase();
  if (s === "active") {
    return "bg-emerald-50 text-emerald-700 border-emerald-200";
  }
  if (s === "retired") {
    return "bg-slate-100 text-slate-700 border-slate-300";
  }
  return "bg-amber-50 text-amber-800 border-amber-200";
}

function subscriptionForLicense(context: CustomerContext, subscriptionId?: string | null): any | null {
  if (!subscriptionId) {
    return null;
  }
  const rows = (context?.subscriptions ?? []) as any[];
  return rows.find((s) => s.subscriptionId === subscriptionId) ?? null;
}

export default function ManagePage() {
  const { theme, setTheme } = useSiteTheme();
  const [email, setEmail] = useState("");
  const [info, setInfo] = useState("");
  const [context, setContext] = useState<CustomerContext | null>(null);
  const [booting, setBooting] = useState(true);
  const [error, setError] = useState("");
  const [actionInfo, setActionInfo] = useState("");
  const accessToken = useMemo(() => {
    if (typeof window === "undefined") {
      return "";
    }
    return new URLSearchParams(window.location.search).get("access") ?? "";
  }, []);

  useEffect(() => {
    async function bootstrap() {
      setError("");
      if (accessToken) {
        const resp = await fetch("/api/customer/access/consume", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ accessToken })
        });
        const payload = await resp.json();
        if (!resp.ok || !payload.ok) {
          setError(payload.error ?? "invalid_access_link");
        } else if (typeof window !== "undefined") {
          window.history.replaceState({}, "", "/manage");
        }
      }
      await loadContext(true);
      setBooting(false);
    }
    void bootstrap();
  }, [accessToken]);

  async function requestAccessLink() {
    setError("");
    setInfo("");
    const resp = await fetch("/api/customer/access/request", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ email })
    });
    const payload = await resp.json();
    if (!resp.ok || !payload.ok) {
      setError(payload.error ?? "access_request_failed");
      return;
    }
    setInfo(payload.message ?? "If this email is valid, a link has been sent.");
  }

  async function loadContext(suppressUnauthed = false) {
    const resp = await fetch("/api/customer/context", { cache: "no-store" });
    const payload = await resp.json();
    if (!resp.ok || !payload.ok) {
      if (
        suppressUnauthed &&
        (payload?.error === "customer_session_required" || payload?.error === "unauthorized")
      ) {
        return;
      }
      setError(payload.error ?? "context_failed");
      return;
    }
    setContext(payload);
  }

  async function retire(bindingId: string) {
    setError("");
    setActionInfo("");
    const resp = await fetch("/api/customer/device/retire", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ bindingId })
    });
    const payload = await resp.json();
    if (!resp.ok || !payload.ok) {
      setError(payload.error ?? "retire_failed");
      return;
    }
    setContext(payload.context ?? context);
    setActionInfo("Seat revoked. Activate LLM Hotspot on a new machine with the same license key to auto-claim the open seat.");
  }

  async function openPortal() {
    const resp = await fetch("/api/customer/portal", { method: "POST" });
    const payload = await resp.json();
    if (!resp.ok || !payload.ok) {
      setError(payload.error ?? "portal_failed");
      return;
    }
    if (payload.portalUrl) {
      window.location.href = payload.portalUrl;
    }
  }

  async function copyText(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      setActionInfo("License key copied.");
    } catch {
      setActionInfo("Unable to copy automatically. Please select and copy manually.");
    }
  }

  return (
    <main className="shell">
      <SiteHeader
        theme={theme}
        setTheme={setTheme}
        links={[
          { label: "Pricing", href: "/#pricing" },
          { label: "Download", href: "/download" }
        ]}
      />
      <h1 className="text-4xl font-black">Customer Seat Management</h1>
      <p className="mt-2" style={{ color: "var(--muted)" }}>Access subscriptions, devices, and billing portal from your email link.</p>

      {!context && !booting ? (
        <div className="panel mt-5 p-5">
          <h2 className="text-lg font-semibold">Email Access Link</h2>
          <div className="mt-3 grid gap-2 md:max-w-lg">
            <input
              className="rounded-xl border border-slate-300 px-3 py-2"
              placeholder="you@company.com"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.currentTarget.value)}
            />
            <button className="btn btn-primary" type="button" onClick={requestAccessLink}>
              Send Access Link
            </button>
          </div>
          {info ? <p className="mt-2 text-sm text-emerald-700">{info}</p> : null}
          {error ? <p className="mt-2 text-sm text-red-700">{error}</p> : null}
        </div>
      ) : null}

      {context ? (
        <div className="panel mt-5 p-5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h2 className="text-lg font-semibold">Licenses and Seats</h2>
            <button className="btn btn-secondary" type="button" onClick={openPortal} disabled={Boolean(context.pending)}>
              Open Billing Portal
            </button>
          </div>
          <div className="mt-3 rounded-xl border p-3 text-sm" style={{ borderColor: "var(--line)", background: "var(--soft)" }}>
            <p>
              <span className="font-semibold">Account email:</span> {context.customer?.email ?? "-"}
            </p>
            <p className="mt-1" style={{ color: "var(--muted)" }}>
              To move a seat: revoke it here, then sign into the app on another machine. The open seat is claimed automatically.
            </p>
          </div>
          {context.pending ? (
            <div className="mt-3 rounded-xl border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">
              Account setup is still pending. This usually means checkout finished recently and billing sync has not completed yet.
              Refresh in a minute, or complete checkout if you have not finished payment.
            </div>
          ) : null}
          {actionInfo ? (
            <div className="mt-3 rounded-xl border border-emerald-200 bg-emerald-50 p-3 text-sm text-emerald-800">
              {actionInfo}
            </div>
          ) : null}
          <div className="mt-4 grid gap-4">
            {(context.licenses ?? []).map((license: any, idx: number) => (
              <div key={license.licenseId} className="rounded-xl border p-4" style={{ borderColor: "var(--line)" }}>
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div>
                    <div className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--muted)" }}>License {idx + 1}</div>
                    <div className="mt-1 flex flex-wrap items-center gap-2">
                      <span className="text-lg font-semibold">{String(license.plan ?? "pro").toUpperCase()}</span>
                      <span className={`rounded-full border px-2 py-0.5 text-xs ${statusPillClass(license.status)}`}>
                        {license.status ?? "-"}
                      </span>
                    </div>
                    <div className="mt-1 text-sm" style={{ color: "var(--muted)" }}>
                      Seats in use: <span className="font-semibold" style={{ color: "var(--ink)" }}>{license.activeSeats}</span> of{" "}
                      <span className="font-semibold" style={{ color: "var(--ink)" }}>{license.seatLimit}</span>
                    </div>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="rounded-lg border px-2 py-1 font-mono text-xs" style={{ borderColor: "var(--line)", background: "var(--soft)", color: "var(--ink)" }}>
                      {shortId(license.licenseId)}
                    </span>
                    <button className="btn btn-secondary" type="button" onClick={() => copyText(license.licenseKey)}>
                      Copy License Key
                    </button>
                  </div>
                </div>
                <div className="mt-3 grid gap-2 text-sm md:grid-cols-2" style={{ color: "var(--ink)" }}>
                  <p>
                    <span className="font-semibold">Purchased:</span> {formatDateOnly(license.createdUnix)}
                  </p>
                  <p>
                    <span className="font-semibold">Last updated:</span> {formatUnix(license.updatedUnix)}
                  </p>
                  <p>
                    <span className="font-semibold">Grace period ends:</span> {formatDateOnly(license.graceExpiresUnix)}
                  </p>
                </div>
                {(() => {
                  const sub = subscriptionForLicense(context, license.subscriptionId);
                  if (!sub) {
                    return null;
                  }
                  return (
                    <div className="mt-2 rounded-lg border px-3 py-2 text-sm" style={{ borderColor: "var(--line)", background: "var(--soft)", color: "var(--ink)" }}>
                      <p>
                        <span className="font-semibold">Subscription:</span>{" "}
                        <span className="font-mono">{shortId(sub.subscriptionId)}</span>
                      </p>
                      <p>
                        <span className="font-semibold">Next renewal:</span> {formatDateOnly(sub.currentPeriodEndUnix)}
                      </p>
                      <p>
                        <span className="font-semibold">Started:</span> {formatDateOnly(sub.createdUnix)}
                      </p>
                    </div>
                  );
                })()}
                <div className="mt-2 grid gap-2">
                  {(license.bindings ?? []).map((binding: any) => (
                    <div key={binding.bindingId}>
                      <div className="flex flex-wrap items-center justify-between rounded-lg border px-3 py-2" style={{ borderColor: "var(--line)" }}>
                        <div className="text-sm">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="font-semibold">{binding.deviceName || shortId(binding.deviceId)}</span>
                            <span className={`rounded-full border px-2 py-0.5 text-xs ${statusPillClass(binding.status)}`}>
                              {binding.status ?? "-"}
                            </span>
                          </div>
                          <span className="text-xs" style={{ color: "var(--muted)" }}>Device ID: {binding.deviceId || "-"}</span>
                          <br />
                          <span className="text-xs" style={{ color: "var(--muted)" }}>
                            First seen: {formatUnix(binding.firstSeenUnix)} | Last seen: {formatUnix(binding.lastSeenUnix)}
                          </span>
                          <br />
                          <span className="text-xs" style={{ color: "var(--muted)" }}>
                            OS: {binding.os || "unknown"} | Country: {binding.countryCode || "unknown"}
                          </span>
                        </div>
                        <div className="flex gap-2">
                          {binding.status === "active" ? (
                            <button className="btn btn-secondary" type="button" onClick={() => retire(binding.bindingId)}>
                              Revoke Seat
                            </button>
                          ) : null}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      ) : null}

      {booting ? <p className="mt-4 text-sm" style={{ color: "var(--muted)" }}>Loading...</p> : null}
    </main>
  );
}
