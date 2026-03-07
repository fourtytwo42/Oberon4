"use client";

import Image from "next/image";
import { useEffect, useState } from "react";
import { SeatPricingCard } from "@/components/seat-pricing-card";
import { SiteHeader } from "@/components/site-header";
import { useSiteTheme } from "@/components/theme-provider";

export default function HomePage() {
  const { theme, setTheme } = useSiteTheme();
  const [toast, setToast] = useState<string>("");

  useEffect(() => {
    if (!toast) {
      return;
    }
    const timeout = window.setTimeout(() => setToast(""), 1700);
    return () => window.clearTimeout(timeout);
  }, [toast]);

  async function copyText(value: string, label: string) {
    try {
      if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(value);
        setToast(`${label} copied`);
      } else {
        setToast(`${label} ready`);
      }
    } catch {
      setToast(`${label} ready`);
    }
  }

  return (
    <main className="shell">
      <SiteHeader
        theme={theme}
        setTheme={setTheme}
        links={[
          { label: "Pricing", href: "#pricing" },
          { label: "Product Tour", href: "#product-tour" },
          { label: "Benefits", href: "#benefits" },
          { label: "Download", href: "/download" }
        ]}
      />

      <section className="grid-2 items-start">
        <div>
          <p className="text-sm font-semibold uppercase tracking-wider" style={{ color: "var(--brand)" }}>
            One Endpoint For Your Entire AI Stack
          </p>
          <h1 className="mt-3 text-5xl font-black leading-tight">
            Run every AI tool through one managed endpoint and stop paying API-sprawl tax.
          </h1>
          <p className="mt-4 max-w-xl text-lg" style={{ color: "var(--muted)" }}>
            LLM Hotspot connects your OpenAI and Anthropic OAuth accounts into a single production-ready endpoint. Your team keeps using the tools they already like, while you reduce API overhead, eliminate account switching, and scale cleanly across engineering and ops.
          </p>
          <div className="mt-5 flex flex-wrap gap-3">
            <a className="btn btn-primary" href="#pricing">Start Subscription</a>
            <a className="btn btn-secondary" href="/manage">Already a customer?</a>
            <a className="btn btn-secondary" href="/download">Download Free App</a>
          </div>
        </div>
        <SeatPricingCard />
      </section>

      <section id="product-tour" className="mt-8 panel p-5">
        <h2 className="text-2xl font-semibold">See the app before you buy</h2>
        <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
          This is a live product-style walkthrough of the exact experience your team gets: endpoint management, OAuth control, and key-based integrations.
        </p>
      </section>

      <section className="mt-4 grid gap-4">
        <div className="showcase-row">
          <div className="panel p-5">
            <p className="text-xs font-semibold uppercase tracking-wider" style={{ color: "var(--brand)" }}>
              Endpoint Surface
            </p>
            <h3 className="mt-2 text-xl font-semibold">Instant local + remote endpoints</h3>
            <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
              Teams get copy-ready URLs for local development, public routing, and remote console control. Drop these directly into AI IDEs, bots, and automations.
            </p>
          </div>
          <article className="hotspot-ui hotspot-mini">
            <h3>Endpoints</h3>
            <div className="hotspot-group">
              <p>LOCAL</p>
              <div className="hotspot-pill">
                <code>http://127.0.0.1:51824/v1</code>
                <button type="button" onClick={() => copyText("http://127.0.0.1:51824/v1", "Local URL")} className="copy-glyph" aria-label="Copy local URL" />
              </div>
            </div>
            <div className="hotspot-group">
              <p>REMOTE</p>
              <div className="hotspot-pill">
                <code>https://team-demo.llmhotspot.example/v1</code>
                <button type="button" onClick={() => copyText("https://team-demo.llmhotspot.example/v1", "Remote URL")} className="copy-glyph" aria-label="Copy remote URL" />
              </div>
            </div>
            <div className="hotspot-group">
              <p>REMOTE CONSOLE</p>
              <div className="hotspot-pill">
                <code>https://team-demo.llmhotspot.example/remote?admin_key=adm_demo_public_preview_key</code>
                <button type="button" onClick={() => copyText("https://team-demo.llmhotspot.example/remote?admin_key=adm_demo_public_preview_key", "Remote console URL")} className="copy-glyph" aria-label="Copy remote console URL" />
              </div>
            </div>
          </article>
        </div>

        <div className="showcase-row">
          <div className="panel p-5">
            <p className="text-xs font-semibold uppercase tracking-wider" style={{ color: "var(--brand)" }}>
              OAuth Routing
            </p>
            <h3 className="mt-2 text-xl font-semibold">Route across multiple provider accounts</h3>
            <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
              Keep multiple OpenAI and Anthropic OAuth accounts connected and prioritized. You get smoother uptime, cost control, and no per-tool account switching.
            </p>
          </div>
          <article className="hotspot-ui hotspot-mini">
            <h3>OAuth Accounts</h3>
            <div className="hotspot-toolbar">
              <input readOnly value="label (optional)" />
              <button type="button" className="hotspot-btn" onClick={() => setToast("OAuth connect preview")}>Connect</button>
              <button type="button" className="hotspot-btn" onClick={() => setToast("Anthropic connect preview")}>AI Connect</button>
            </div>

            <div className="hotspot-account">
              <div className="hotspot-account-head">
                <div>
                  <strong>atlas-eng-primary</strong>
                  <span className="hotspot-badge">plus</span>
                </div>
                <div className="hotspot-actions">
                  <button type="button" className="ghost-arrow" aria-label="Move account down">↓</button>
                  <button type="button" className="toggle-on" aria-label="Toggle account" />
                  <button type="button" className="ghost-danger" aria-label="Delete account" />
                </div>
              </div>
              <div className="hotspot-chip">OpenAI OAuth</div>
              <div className="meter-row">
                <span>5H</span>
                <div className="meter"><div style={{ width: "71%" }} /></div>
                <small>71% · 1h 57m</small>
              </div>
              <div className="meter-row">
                <span>WEEKLY</span>
                <div className="meter amber"><div style={{ width: "33%" }} /></div>
                <small>33% · 5d 15h</small>
              </div>
              <div className="oauth-token-usage">
                <div className="usage-title-row">
                  <span>▾ TOKEN USAGE</span>
                  <button type="button" className="usage-reset-btn" onClick={() => setToast("OAuth usage reset preview")}>Reset</button>
                </div>
                <p className="usage-head"><span>MODEL</span><span>IN</span><span>OUT</span><span>CAC</span></p>
                <p className="usage-row"><strong>gpt-5.3-codex</strong><span>23.60M</span><span>124.8K</span><span>21.99M</span></p>
              </div>
            </div>

            <div className="hotspot-account">
              <div className="hotspot-account-head">
                <div>
                  <strong>nova-research-fallback</strong>
                  <span className="hotspot-badge">plus</span>
                </div>
                <div className="hotspot-actions">
                  <button type="button" className="ghost-arrow" aria-label="Move account up">↑</button>
                  <button type="button" className="ghost-arrow" aria-label="Move account down">↓</button>
                  <button type="button" className="toggle-on" aria-label="Toggle account" />
                  <button type="button" className="ghost-danger" aria-label="Delete account" />
                </div>
              </div>
              <div className="hotspot-chip">Anthropic OAuth</div>
              <div className="meter-row">
                <span>5H</span>
                <div className="meter"><div style={{ width: "100%" }} /></div>
                <small>100% · 5h</small>
              </div>
              <div className="meter-row">
                <span>WEEKLY</span>
                <div className="meter"><div style={{ width: "100%" }} /></div>
                <small>100% · 7d</small>
              </div>
            </div>
          </article>
        </div>

        <div className="showcase-row">
          <div className="panel p-5">
            <p className="text-xs font-semibold uppercase tracking-wider" style={{ color: "var(--brand)" }}>
              API Key Layer
            </p>
            <h3 className="mt-2 text-xl font-semibold">Unified keys with usage visibility</h3>
            <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
              Keep existing OpenAI-compatible clients working while you rotate keys, inspect usage, and enforce team-level control from one place.
            </p>
          </div>
          <article className="hotspot-ui hotspot-mini">
            <h3>API Keys</h3>
            <div className="hotspot-toolbar">
              <input readOnly value="label (optional)" />
              <input readOnly value="mm/dd/yyyy --:-- --" />
              <button type="button" className="hotspot-btn" onClick={() => setToast("Create key preview")}>Create Key</button>
            </div>

            <div className="hotspot-account">
              <div className="hotspot-account-head">
                <div>
                  <strong>default</strong>
                  <span className="hotspot-status">active</span>
                </div>
                <div className="hotspot-actions">
                  <button type="button" className="toggle-on" aria-label="Toggle key" />
                  <button type="button" className="ghost-danger" aria-label="Delete key" />
                </div>
              </div>
              <div className="hotspot-pill">
                <code>hs_demo_preview_key_for_ui_only</code>
                <button type="button" onClick={() => copyText("hs_demo_preview_key_for_ui_only", "API key")} className="copy-glyph" aria-label="Copy API key" />
              </div>
              <div className="hotspot-details">
                <div>
                  <p>CREATED <strong>2/24/2026, 1:30:36 AM</strong></p>
                  <p>EXPIRES <strong>-</strong></p>
                  <p>USED <strong>2/27/2026, 12:45:25 AM</strong></p>
                </div>
                <div>
                  <p>TOKEN USAGE</p>
                  <p className="usage-head"><span>MODEL</span><span>IN</span><span>OUT</span><span>CAC</span></p>
                  <p className="usage-row"><strong>gpt-5.3-codex</strong><span>24.02M</span><span>125.5K</span><span>22.36M</span></p>
                </div>
              </div>
            </div>
          </article>
        </div>

        <div className="showcase-row">
          <div className="panel p-5">
            <p className="text-xs font-semibold uppercase tracking-wider" style={{ color: "var(--brand)" }}>
              Model Catalog
            </p>
            <h3 className="mt-2 text-xl font-semibold">One endpoint, many model targets</h3>
            <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
              Expose broad model support to your team behind a single base URL, with usage counters ready for governance and budgeting.
            </p>
          </div>
          <article className="hotspot-ui hotspot-mini">
            <h3>Supported Models</h3>
            <div className="models-head">
              <span>MODEL</span><span>IN</span><span>OUT</span><span>CAC</span>
            </div>
            {[
              "gpt-5.3-codex",
              "gpt-5.3-codex-spark",
              "gpt-5.2-codex",
              "gpt-5.2",
              "gpt-5.1-codex-max",
              "gpt-5.1-codex",
              "claude-opus-4-6",
              "claude-sonnet-4-6",
              "claude-opus-4-5",
              "claude-sonnet-4-5"
            ].map((model, idx) => (
              <div className={`model-row ${idx === 0 ? "active" : ""}`} key={model}>
                <button type="button" onClick={() => copyText(model, "Model name")} className="model-name">{model}</button>
                <span>{idx === 0 ? "24.02M" : "—"}</span>
                <span>{idx === 0 ? "125.5K" : "—"}</span>
                <span>{idx === 0 ? "22.36M" : "—"}</span>
              </div>
            ))}
            <div className="models-footer">
              <button type="button" onClick={() => setToast("Model counters reset preview")} className="reset-chip">Reset Model Counters</button>
            </div>
          </article>
        </div>
      </section>

      <section className="mt-6 panel p-5">
        <h2 className="text-2xl font-semibold">Free vs Paid Seats</h2>
        <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
          Start free, then scale into paid seats when you need team-grade operations.
        </p>
        <div className="mt-4 grid gap-3 md:grid-cols-2">
          <div className="rounded-xl border px-4 py-4" style={{ borderColor: "var(--line)" }}>
            <h3 className="font-semibold">Free App</h3>
            <ul className="mt-2 grid gap-2 text-sm" style={{ color: "var(--muted)" }}>
              <li>Run locally with no subscription required.</li>
              <li>Great for personal workflows and evaluation.</li>
              <li>Use local endpoint behavior to validate integrations.</li>
            </ul>
          </div>
          <div className="rounded-xl border px-4 py-4" style={{ borderColor: "var(--line)" }}>
            <h3 className="font-semibold">Paid Seats</h3>
            <ul className="mt-2 grid gap-2 text-sm" style={{ color: "var(--muted)" }}>
              <li>Managed remote tunnels and seat-level capacity.</li>
              <li>Customer seat operations plus admin controls.</li>
              <li>Team-ready billing, routing, and centralized governance.</li>
            </ul>
          </div>
        </div>
      </section>

      <section id="benefits" className="mt-8 panel p-5">
        <h2 className="text-2xl font-semibold">Why teams switch to LLM Hotspot</h2>
        <div className="mt-4 grid gap-3 md:grid-cols-3">
          <div className="rounded-xl border px-4 py-4" style={{ borderColor: "var(--line)" }}>
            <h3 className="font-semibold">Lower Monthly Spend</h3>
            <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
              Stop routing every workflow through expensive direct API usage. Use OAuth-backed access where it makes sense and turn uncontrolled per-tool spend into a predictable operating model.
            </p>
          </div>
          <div className="rounded-xl border px-4 py-4" style={{ borderColor: "var(--line)" }}>
            <h3 className="font-semibold">One Endpoint Across Accounts</h3>
            <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
              Consolidate multiple OpenAI and Anthropic OAuth accounts under one endpoint so developers, agents, and automations never need account-by-account reconfiguration.
            </p>
          </div>
          <div className="rounded-xl border px-4 py-4" style={{ borderColor: "var(--line)" }}>
            <h3 className="font-semibold">Works With Real Toolchains</h3>
            <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
              Use the same endpoint in AI IDEs, coding assistants, chat bots, OpenClaw-style systems, and internal agents without rewiring each product when your stack evolves.
            </p>
          </div>
        </div>
      </section>

      <section className="mt-8 panel p-5">
        <h2 className="text-xl font-semibold">What this unlocks for your team</h2>
        <ul className="mt-3 grid gap-2 text-sm md:grid-cols-2" style={{ color: "var(--muted)" }}>
          <li className="rounded-xl border px-3 py-3" style={{ borderColor: "var(--line)" }}>Give developers one stable endpoint for IDE copilots, terminals, and agent tooling.</li>
          <li className="rounded-xl border px-3 py-3" style={{ borderColor: "var(--line)" }}>Deploy faster because new tools inherit your endpoint instead of requiring new account wiring.</li>
          <li className="rounded-xl border px-3 py-3" style={{ borderColor: "var(--line)" }}>Keep teams productive when models or providers change by swapping backend auth, not client configs.</li>
          <li className="rounded-xl border px-3 py-3" style={{ borderColor: "var(--line)" }}>Support multi-account routing for departments, projects, or clients behind a single control plane.</li>
          <li className="rounded-xl border px-3 py-3" style={{ borderColor: "var(--line)" }}>Reduce support load from broken tokens, account drift, and per-tool credential setup.</li>
          <li className="rounded-xl border px-3 py-3" style={{ borderColor: "var(--line)" }}>Turn fragmented AI access into an enterprise-ready layer you can manage and scale confidently.</li>
        </ul>
      </section>

      <section className="mt-8 panel p-5">
        <h2 className="text-2xl font-semibold">Download LLM Hotspot Free</h2>
        <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
          Start with the free app, connect your tools, and upgrade only when you need managed seats, team controls, and full production operations.
        </p>
        <div className="mt-4 flex flex-wrap gap-3">
          <a className="btn btn-primary" href="/download">Download Free App</a>
          <a className="btn btn-secondary" href="#pricing">Upgrade to Team Plan</a>
        </div>
      </section>
      {toast ? <div className="demo-toast">{toast}</div> : null}
    </main>
  );
}
