"use client";

import { useState } from "react";
import { SiteHeader } from "@/components/site-header";
import { useSiteTheme } from "@/components/theme-provider";

export default function ContactPage() {
  const { theme, setTheme } = useSiteTheme();
  const [email, setEmail] = useState("");
  const [company, setCompany] = useState("");
  const [seats, setSeats] = useState(150);
  const [note, setNote] = useState("");
  const [status, setStatus] = useState("");

  async function submit() {
    setStatus("");
    const resp = await fetch("/api/contact", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ email, company, seats, note })
    });
    const payload = await resp.json();
    if (!resp.ok || !payload.ok) {
      setStatus(payload.error ?? "request_failed");
      return;
    }
    setStatus("Request sent. We will contact you.");
  }

  return (
    <main className="shell md:max-w-3xl">
      <SiteHeader
        theme={theme}
        setTheme={setTheme}
        links={[
          { label: "Pricing", href: "/#pricing" },
          { label: "Manage Seats", href: "/manage" }
        ]}
      />

      <div className="panel p-6 md:p-8">
        <p className="text-sm font-semibold uppercase tracking-wider" style={{ color: "var(--brand)" }}>
          Sales
        </p>
        <h1 className="mt-2 text-4xl font-black">Contact Sales</h1>
        <p className="mt-2" style={{ color: "var(--muted)" }}>
          For more than 100 seats, enterprise controls, custom procurement, or rollout support.
        </p>

        <div className="mt-6 grid gap-3">
          <input
            className="rounded-xl border border-slate-300 px-3 py-2"
            placeholder="Email"
            value={email}
            onChange={(e) => setEmail(e.currentTarget.value)}
          />
          <input
            className="rounded-xl border border-slate-300 px-3 py-2"
            placeholder="Company"
            value={company}
            onChange={(e) => setCompany(e.currentTarget.value)}
          />
          <input
            className="rounded-xl border border-slate-300 px-3 py-2"
            type="number"
            min={101}
            value={seats}
            onChange={(e) => setSeats(Number(e.currentTarget.value))}
          />
          <textarea
            className="rounded-xl border border-slate-300 px-3 py-2"
            rows={4}
            placeholder="Anything we should know?"
            value={note}
            onChange={(e) => setNote(e.currentTarget.value)}
          />
          <div className="flex flex-wrap items-center gap-2">
            <button className="btn btn-primary" type="button" onClick={submit}>
              Send
            </button>
            {status ? <p className="text-sm" style={{ color: "var(--muted)" }}>{status}</p> : null}
          </div>
        </div>
      </div>
    </main>
  );
}
