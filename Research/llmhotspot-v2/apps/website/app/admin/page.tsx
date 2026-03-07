"use client";

import { useEffect, useMemo, useState } from "react";
import { SiteHeader } from "@/components/site-header";
import { useSiteTheme } from "@/components/theme-provider";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis
} from "recharts";

type TabKey =
  | "overview"
  | "customers"
  | "subscriptions"
  | "devices"
  | "billing"
  | "reports"
  | "health"
  | "smtp";

const TABS: Array<{ key: TabKey; label: string }> = [
  { key: "overview", label: "Overview" },
  { key: "customers", label: "Customers" },
  { key: "subscriptions", label: "Subscriptions" },
  { key: "devices", label: "Devices" },
  { key: "billing", label: "Billing" },
  { key: "reports", label: "Reports" },
  { key: "health", label: "System Health" },
  { key: "smtp", label: "SMTP" }
];

type AdminState = {
  metrics?: any;
  analytics?: any;
  tree?: any[];
  customers?: any[];
  subscriptions?: any[];
  seats?: any[];
  health?: any;
};

function q(name: string, fallback = ""): string {
  if (typeof window === "undefined") return fallback;
  return new URLSearchParams(window.location.search).get(name) ?? fallback;
}

function withQs(tab: TabKey, period: string): string {
  const p = new URLSearchParams();
  p.set("tab", tab);
  p.set("period", period);
  return `/admin?${p.toString()}`;
}

function extractApiError(payload: any, fallback: string): string {
  if (!payload || typeof payload !== "object") return fallback;
  return payload.error || payload.message || fallback;
}

async function fetchJsonSafe(input: RequestInfo | URL, init?: RequestInit): Promise<{
  resp: Response;
  payload: any;
}> {
  const resp = await fetch(input, init);
  const raw = await resp.text();
  if (!raw || !raw.trim()) {
    return { resp, payload: { ok: false, error: "empty_response" } };
  }
  try {
    return { resp, payload: JSON.parse(raw) };
  } catch {
    return { resp, payload: { ok: false, error: "invalid_json_response" } };
  }
}

function moneyCents(v?: number): string {
  const n = Number(v ?? 0);
  return `$${(n / 100).toLocaleString(undefined, { maximumFractionDigits: 2 })}`;
}

function formatUnix(unix?: number): string {
  if (!unix) return "-";
  return new Date(unix * 1000).toLocaleString();
}

function shortId(raw?: string): string {
  const s = String(raw ?? "");
  if (!s) return "-";
  if (s.length <= 16) return s;
  return `${s.slice(0, 8)}...${s.slice(-6)}`;
}

function containsText(value: unknown, query: string): boolean {
  if (!query) return true;
  return String(value ?? "").toLowerCase().includes(query.toLowerCase());
}

export default function AdminPage() {
  const { theme, setTheme } = useSiteTheme();
  const [tab, setTab] = useState<TabKey>("overview");
  const [period, setPeriod] = useState("1w");
  const [authed, setAuthed] = useState(false);
  const [adminSecret, setAdminSecret] = useState("");
  const [state, setState] = useState<AdminState>({});
  const [error, setError] = useState("");
  const [info, setInfo] = useState("");

  const [smtp, setSmtp] = useState({
    host: "",
    port: 587,
    secure: false,
    username: "",
    password: "",
    fromEmail: "",
    fromName: ""
  });
  const [smtpPasswordSet, setSmtpPasswordSet] = useState(false);
  const [smtpTestEmail, setSmtpTestEmail] = useState("");
  const [customerQuery, setCustomerQuery] = useState("");
  const [customerStatusFilter, setCustomerStatusFilter] = useState("all");
  const [customerSort, setCustomerSort] = useState("email_asc");
  const [subscriptionQuery, setSubscriptionQuery] = useState("");
  const [subscriptionStatusFilter, setSubscriptionStatusFilter] = useState("all");
  const [subscriptionSort, setSubscriptionSort] = useState("renewal_desc");
  const [deviceQuery, setDeviceQuery] = useState("");
  const [deviceStatusFilter, setDeviceStatusFilter] = useState("all");
  const [deviceSort, setDeviceSort] = useState("last_seen_desc");

  useEffect(() => {
    const t = q("tab", "overview") as TabKey;
    const p = q("period", "1w");
    setTab(TABS.some((x) => x.key === t) ? t : "overview");
    setPeriod(["1d", "1w", "1m", "1y"].includes(p) ? p : "1w");
  }, []);

  async function login() {
    setError("");
    const { resp, payload } = await fetchJsonSafe("/api/admin/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ adminSecret })
    });
    if (!resp.ok || !payload.ok) {
      setError(extractApiError(payload, "login_failed"));
      return;
    }
    setAuthed(true);
    await loadAll(period);
  }

  async function restoreAdminSession() {
    try {
      const { resp, payload } = await fetchJsonSafe("/api/admin/metrics", { cache: "no-store" });
      if (resp.ok && payload?.ok) {
        setAuthed(true);
        await loadAll(period);
      }
    } catch {
      // no-op
    }
  }

  useEffect(() => {
    void restoreAdminSession();
  }, []);

  async function loadAll(currentPeriod: string) {
    setError("");
    const [metricsR, analyticsR, treeR, customersR, subsR, seatsR, healthR, smtpR] = await Promise.all([
      fetchJsonSafe("/api/admin/metrics", { cache: "no-store" }),
      fetchJsonSafe(`/api/admin/analytics/overview?period=${encodeURIComponent(currentPeriod)}`, {
        cache: "no-store"
      }),
      fetchJsonSafe(`/api/admin/customer-tree?period=${encodeURIComponent(currentPeriod)}`, { cache: "no-store" }),
      fetchJsonSafe(`/api/admin/customers?period=${encodeURIComponent(currentPeriod)}`, { cache: "no-store" }),
      fetchJsonSafe(`/api/admin/subscriptions?period=${encodeURIComponent(currentPeriod)}`, { cache: "no-store" }),
      fetchJsonSafe("/api/admin/seats", { cache: "no-store" }),
      fetchJsonSafe("/api/admin/health", { cache: "no-store" }),
      fetchJsonSafe("/api/admin/smtp", { cache: "no-store" })
    ]);
    const responses = [metricsR, analyticsR, treeR, customersR, subsR, seatsR, healthR, smtpR];
    const unauthorized = responses.some((r) => r.resp.status === 401);
    if (unauthorized) {
      setAuthed(false);
      setError("admin_session_expired");
      return;
    }

    const metrics = metricsR.payload ?? {};
    const analytics = analyticsR.payload ?? {};
    const tree = treeR.payload ?? {};
    const customers = customersR.payload ?? {};
    const subs = subsR.payload ?? {};
    const seats = seatsR.payload ?? {};
    const health = healthR.payload ?? {};
    const smtpResp = smtpR.payload ?? {};

    if (smtpResp?.ok && smtpResp.settings) {
      setSmtp({
        host: smtpResp.settings.host ?? "",
        port: smtpResp.settings.port ?? 587,
        secure: Boolean(smtpResp.settings.secure),
        username: smtpResp.settings.username ?? "",
        password: "",
        fromEmail: smtpResp.settings.fromEmail ?? "",
        fromName: smtpResp.settings.fromName ?? ""
      });
      setSmtpPasswordSet(Boolean(smtpResp.settings.passwordSet));
    }
    setState({
      metrics: metrics.metrics ?? {},
      analytics,
      tree: tree.items ?? [],
      customers: customers.items ?? [],
      subscriptions: subs.items ?? [],
      seats: seats.items ?? [],
      health: health.health ?? {}
    });
  }

  async function setTabAndUrl(next: TabKey) {
    setTab(next);
    if (typeof window !== "undefined") {
      window.history.replaceState({}, "", withQs(next, period));
    }
  }

  async function setPeriodAndReload(next: string) {
    setPeriod(next);
    if (typeof window !== "undefined") {
      window.history.replaceState({}, "", withQs(tab, next));
    }
    if (authed) await loadAll(next);
  }

  async function suspend(subscriptionId: string) {
    setError("");
    const { resp, payload } = await fetchJsonSafe(
      `/api/admin/subscriptions/${encodeURIComponent(subscriptionId)}/suspend`,
      {
      method: "POST"
      }
    );
    if (!resp.ok || !payload.ok) {
      setError(extractApiError(payload, "suspend_failed"));
      return;
    }
    setInfo(`Suspended ${subscriptionId}`);
    await loadAll(period);
  }

  async function unsuspend(subscriptionId: string) {
    setError("");
    const { resp, payload } = await fetchJsonSafe(
      `/api/admin/subscriptions/${encodeURIComponent(subscriptionId)}/unsuspend`,
      {
      method: "POST"
      }
    );
    if (!resp.ok || !payload.ok) {
      setError(extractApiError(payload, "unsuspend_failed"));
      return;
    }
    setInfo(`Unsuspended ${subscriptionId}`);
    await loadAll(period);
  }

  async function exportCsv(kind: "customers" | "subscriptions" | "devices" | "revenue") {
    const qs = kind === "revenue" ? `?period=${encodeURIComponent(period)}` : "";
    window.open(`/api/admin/export/${kind}.csv${qs}`, "_blank", "noopener,noreferrer");
  }

  async function saveSmtp() {
    setError("");
    setInfo("");
    const { resp, payload } = await fetchJsonSafe("/api/admin/smtp", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        host: smtp.host,
        port: Number(smtp.port),
        secure: smtp.secure,
        username: smtp.username,
        password: smtp.password || undefined,
        fromEmail: smtp.fromEmail,
        fromName: smtp.fromName || undefined
      })
    });
    if (!resp.ok || !payload.ok) {
      setError(extractApiError(payload, "smtp_save_failed"));
      return;
    }
    setSmtp((p) => ({ ...p, password: "" }));
    setSmtpPasswordSet(true);
    setInfo("SMTP settings saved.");
  }

  async function sendSmtpTest() {
    setError("");
    setInfo("");
    const { resp, payload } = await fetchJsonSafe("/api/admin/smtp/test", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        testEmail: smtpTestEmail,
        host: smtp.host,
        port: Number(smtp.port),
        secure: smtp.secure,
        username: smtp.username,
        password: smtp.password || undefined,
        fromEmail: smtp.fromEmail,
        fromName: smtp.fromName || undefined
      })
    });
    if (!resp.ok || !payload.ok) {
      setError(extractApiError(payload, "smtp_test_failed"));
      return;
    }
    setInfo(`Test email sent to ${smtpTestEmail}.`);
  }

  const totals = state.analytics?.totals ?? {};
  const series = state.analytics?.series ?? {};
  const filteredCustomers = useMemo(() => {
    const rows = [...(state.tree ?? [])];
    const query = customerQuery.trim().toLowerCase();
    const filtered = rows.filter((customer: any) => {
      const subscriptions = customer.subscriptions ?? [];
      const statusMatch =
        customerStatusFilter === "all" ||
        subscriptions.some((s: any) => String(s.status ?? "").toLowerCase() === customerStatusFilter);
      if (!statusMatch) return false;
      if (!query) return true;
      if (containsText(customer.email, query) || containsText(customer.customerId, query)) return true;
      return subscriptions.some((s: any) => {
        if (containsText(s.subscriptionId, query) || containsText(s.status, query)) return true;
        return (s.licenses ?? []).some((l: any) => {
          if (containsText(l.licenseId, query) || containsText(l.status, query)) return true;
          return (l.bindings ?? []).some((b: any) =>
            [b.deviceName, b.deviceId, b.os, b.countryCode].some((v: any) => containsText(v, query))
          );
        });
      });
    });
    filtered.sort((a: any, b: any) => {
      const aSubs = (a.subscriptions ?? []).length;
      const bSubs = (b.subscriptions ?? []).length;
      const aSeats = (a.subscriptions ?? []).reduce((sum: number, s: any) => sum + Number(s.seatLimit ?? 0), 0);
      const bSeats = (b.subscriptions ?? []).reduce((sum: number, s: any) => sum + Number(s.seatLimit ?? 0), 0);
      if (customerSort === "email_desc") return String(b.email ?? "").localeCompare(String(a.email ?? ""));
      if (customerSort === "subscriptions_desc") return bSubs - aSubs;
      if (customerSort === "seats_desc") return bSeats - aSeats;
      if (customerSort === "subscriptions_asc") return aSubs - bSubs;
      return String(a.email ?? "").localeCompare(String(b.email ?? ""));
    });
    return filtered;
  }, [state.tree, customerQuery, customerStatusFilter, customerSort]);

  const filteredSubscriptions = useMemo(() => {
    const rows = [...(state.subscriptions ?? [])];
    const query = subscriptionQuery.trim().toLowerCase();
    const filtered = rows.filter((s: any) => {
      const statusOk =
        subscriptionStatusFilter === "all" ||
        String(s.status ?? "").toLowerCase() === subscriptionStatusFilter;
      if (!statusOk) return false;
      if (!query) return true;
      return (
        containsText(s.email, query) ||
        containsText(s.subscriptionId, query) ||
        containsText(s.status, query)
      );
    });
    filtered.sort((a: any, b: any) => {
      if (subscriptionSort === "email_asc") {
        return String(a.email ?? "").localeCompare(String(b.email ?? ""));
      }
      if (subscriptionSort === "email_desc") {
        return String(b.email ?? "").localeCompare(String(a.email ?? ""));
      }
      if (subscriptionSort === "seats_desc") {
        return Number(b.seatLimit ?? 0) - Number(a.seatLimit ?? 0);
      }
      if (subscriptionSort === "seats_asc") {
        return Number(a.seatLimit ?? 0) - Number(b.seatLimit ?? 0);
      }
      if (subscriptionSort === "renewal_asc") {
        return Number(a.currentPeriodEndUnix ?? 0) - Number(b.currentPeriodEndUnix ?? 0);
      }
      return Number(b.currentPeriodEndUnix ?? 0) - Number(a.currentPeriodEndUnix ?? 0);
    });
    return filtered;
  }, [state.subscriptions, subscriptionQuery, subscriptionStatusFilter, subscriptionSort]);

  const filteredDevices = useMemo(() => {
    const rows = [...(state.seats ?? [])];
    const query = deviceQuery.trim().toLowerCase();
    const filtered = rows.filter((d: any) => {
      const statusOk =
        deviceStatusFilter === "all" || String(d.status ?? "").toLowerCase() === deviceStatusFilter;
      if (!statusOk) return false;
      if (!query) return true;
      return [d.deviceName, d.deviceId, d.os, d.countryCode, d.status]
        .some((v: any) => containsText(v, query));
    });
    filtered.sort((a: any, b: any) => {
      if (deviceSort === "device_asc") {
        return String(a.deviceName || a.deviceId || "").localeCompare(String(b.deviceName || b.deviceId || ""));
      }
      if (deviceSort === "device_desc") {
        return String(b.deviceName || b.deviceId || "").localeCompare(String(a.deviceName || a.deviceId || ""));
      }
      if (deviceSort === "os_asc") return String(a.os ?? "").localeCompare(String(b.os ?? ""));
      if (deviceSort === "country_asc") return String(a.countryCode ?? "").localeCompare(String(b.countryCode ?? ""));
      if (deviceSort === "last_seen_asc") return Number(a.lastSeenUnix ?? 0) - Number(b.lastSeenUnix ?? 0);
      return Number(b.lastSeenUnix ?? 0) - Number(a.lastSeenUnix ?? 0);
    });
    return filtered;
  }, [state.seats, deviceQuery, deviceStatusFilter, deviceSort]);

  return (
    <main className="shell">
      <SiteHeader
        theme={theme}
        setTheme={setTheme}
        links={[{ label: "Dashboard", href: "/admin" }, { label: "Site", href: "/" }]}
      />

      <h1 className="text-4xl font-black">Admin Control Plane</h1>
      <p className="mt-2" style={{ color: "var(--muted)" }}>
        Business, entitlement, and system health in one view.
      </p>

      {!authed ? (
        <div className="panel mt-5 p-5 md:max-w-lg">
          <input
            className="w-full rounded-xl border border-slate-300 px-3 py-2"
            placeholder="admin secret"
            value={adminSecret}
            onChange={(e) => setAdminSecret(e.currentTarget.value)}
            type="password"
          />
          <button className="btn btn-primary mt-3" type="button" onClick={login}>
            Login
          </button>
          {error ? <p className="mt-2 text-sm text-red-700">{error}</p> : null}
        </div>
      ) : (
        <>
          <div className="mt-5 flex flex-wrap items-center gap-2">
            {TABS.map((t) => (
              <button
                key={t.key}
                className={`btn ${tab === t.key ? "btn-primary" : "btn-secondary"}`}
                type="button"
                onClick={() => void setTabAndUrl(t.key)}
              >
                {t.label}
              </button>
            ))}
            <div className="ml-auto flex gap-2">
              {(["1d", "1w", "1m", "1y"] as const).map((p) => (
                <button
                  key={p}
                  className={`btn ${period === p ? "btn-primary" : "btn-secondary"}`}
                  type="button"
                  onClick={() => void setPeriodAndReload(p)}
                >
                  {p}
                </button>
              ))}
            </div>
          </div>

          {error ? <p className="mt-3 text-sm text-red-700">{error}</p> : null}
          {info ? <p className="mt-3 text-sm text-emerald-700">{info}</p> : null}

          {tab === "overview" ? (
            <section className="mt-4 grid gap-4">
              <div className="grid gap-3 md:grid-cols-3 xl:grid-cols-6">
                <div className="panel p-4"><p className="text-xs uppercase" style={{ color: "var(--muted)" }}>Total Customers</p><p className="text-2xl font-bold">{Number(totals.total_customers ?? state.metrics?.total_customers ?? 0).toLocaleString()}</p></div>
                <div className="panel p-4"><p className="text-xs uppercase" style={{ color: "var(--muted)" }}>Total Seats</p><p className="text-2xl font-bold">{Number(totals.total_seats ?? state.metrics?.total_seats ?? 0).toLocaleString()}</p></div>
                <div className="panel p-4"><p className="text-xs uppercase" style={{ color: "var(--muted)" }}>Avg Seats / Customer</p><p className="text-2xl font-bold">{Number(totals.avg_seats_per_customer ?? state.metrics?.avg_seats_per_customer ?? 0).toFixed(2)}</p></div>
                <div className="panel p-4"><p className="text-xs uppercase" style={{ color: "var(--muted)" }}>Contracted MRR</p><p className="text-2xl font-bold">{moneyCents(totals.contracted_mrr_cents ?? state.metrics?.contracted_mrr_cents)}</p></div>
                <div className="panel p-4"><p className="text-xs uppercase" style={{ color: "var(--muted)" }}>Cash (Period)</p><p className="text-2xl font-bold">{moneyCents(totals.collected_cash_cents_period)}</p></div>
                <div className="panel p-4"><p className="text-xs uppercase" style={{ color: "var(--muted)" }}>Health</p><p className="text-2xl font-bold">{state.health?.broker_up ? "Healthy" : "Degraded"}</p></div>
              </div>

              <div className="grid gap-4 lg:grid-cols-2">
                <div className="panel p-4" style={{ height: 280 }}>
                  <h3 className="font-semibold">Revenue Trend</h3>
                  <ResponsiveContainer width="100%" height="90%">
                    <AreaChart data={series.revenue_over_time ?? []}>
                      <CartesianGrid strokeDasharray="3 3" />
                      <XAxis dataKey="ts" tickFormatter={(v) => new Date(v * 1000).toLocaleDateString()} />
                      <YAxis />
                      <Tooltip labelFormatter={(v) => formatUnix(Number(v))} />
                      <Area type="monotone" dataKey="value" stroke="#14b8a6" fill="#14b8a633" />
                    </AreaChart>
                  </ResponsiveContainer>
                </div>
                <div className="panel p-4" style={{ height: 280 }}>
                  <h3 className="font-semibold">Seats Growth</h3>
                  <ResponsiveContainer width="100%" height="90%">
                    <BarChart data={series.seats_over_time ?? []}>
                      <CartesianGrid strokeDasharray="3 3" />
                      <XAxis dataKey="ts" tickFormatter={(v) => new Date(v * 1000).toLocaleDateString()} />
                      <YAxis />
                      <Tooltip labelFormatter={(v) => formatUnix(Number(v))} />
                      <Bar dataKey="value" fill="#0ea5e9" />
                    </BarChart>
                  </ResponsiveContainer>
                </div>
              </div>
            </section>
          ) : null}

          {tab === "customers" ? (
            <section className="mt-4 grid gap-3">
              <div className="panel p-4">
                <div className="grid gap-2 md:grid-cols-4">
                  <input
                    className="rounded-xl border border-slate-300 px-3 py-2"
                    placeholder="Search email, subscription, device..."
                    value={customerQuery}
                    onChange={(e) => setCustomerQuery(e.currentTarget.value)}
                  />
                  <select
                    className="admin-select rounded-xl border border-slate-300 px-3 py-2 bg-transparent"
                    value={customerStatusFilter}
                    onChange={(e) => setCustomerStatusFilter(e.currentTarget.value)}
                  >
                    <option value="all">All statuses</option>
                    <option value="active">Active</option>
                    <option value="suspended">Suspended</option>
                    <option value="canceled">Canceled</option>
                  </select>
                  <select
                    className="admin-select rounded-xl border border-slate-300 px-3 py-2 bg-transparent"
                    value={customerSort}
                    onChange={(e) => setCustomerSort(e.currentTarget.value)}
                  >
                    <option value="email_asc">Sort: Email A-Z</option>
                    <option value="email_desc">Sort: Email Z-A</option>
                    <option value="subscriptions_desc">Sort: Most subscriptions</option>
                    <option value="subscriptions_asc">Sort: Fewest subscriptions</option>
                    <option value="seats_desc">Sort: Most seats</option>
                  </select>
                  <div className="flex items-center text-sm" style={{ color: "var(--muted)" }}>
                    Showing {filteredCustomers.length.toLocaleString()} customers
                  </div>
                </div>
              </div>
              <div className="grid gap-3 overflow-y-auto pr-1" style={{ maxHeight: "68vh" }}>
              {filteredCustomers.map((c: any) => (
                <div key={c.customerId} className="panel p-4">
                  <h3 className="text-lg font-semibold">{c.email}</h3>
                  <p className="text-xs" style={{ color: "var(--muted)" }}>Customer ID: {c.customerId}</p>
                  <div className="mt-3 grid gap-3">
                    {(c.subscriptions ?? []).map((s: any) => (
                      <div key={s.subscriptionId} className="rounded-xl border p-3" style={{ borderColor: "var(--line)" }}>
                        <div className="flex items-center justify-between gap-2">
                          <div>
                            <p className="font-semibold">{shortId(s.subscriptionId)} • {s.status}</p>
                            <p className="text-xs" style={{ color: "var(--muted)" }}>Seats: {s.seatLimit}</p>
                          </div>
                          {s.status === "suspended" ? (
                            <button className="btn btn-secondary" onClick={() => void unsuspend(s.subscriptionId)} type="button">Unsuspend</button>
                          ) : (
                            <button className="btn btn-secondary" onClick={() => void suspend(s.subscriptionId)} type="button">Suspend</button>
                          )}
                        </div>
                        {(s.licenses ?? []).map((l: any) => (
                          <div key={l.licenseId} className="mt-2 rounded-lg border p-2" style={{ borderColor: "var(--line)" }}>
                            <p className="text-sm font-semibold">License {shortId(l.licenseId)} ({l.status})</p>
                            <div className="mt-2 grid gap-2 md:grid-cols-2">
                              {(l.bindings ?? []).map((b: any) => (
                                <div key={b.bindingId} className="rounded-lg border p-2" style={{ borderColor: "var(--line)" }}>
                                  <p className="text-sm font-semibold">{b.deviceName || shortId(b.deviceId)}</p>
                                  <p className="text-xs" style={{ color: "var(--muted)" }}>{b.os || "unknown"} • {b.countryCode || "unknown"}</p>
                                  <p className="text-xs" style={{ color: "var(--muted)" }}>Last seen: {formatUnix(b.lastSeenUnix)}</p>
                                </div>
                              ))}
                            </div>
                          </div>
                        ))}
                      </div>
                    ))}
                  </div>
                </div>
              ))}
              </div>
            </section>
          ) : null}

          {tab === "subscriptions" ? (
            <section className="mt-4 panel p-4">
              <h3 className="font-semibold">Subscriptions</h3>
              <div className="mt-3 grid gap-2 md:grid-cols-4">
                <input
                  className="rounded-xl border border-slate-300 px-3 py-2"
                  placeholder="Search email or subscription..."
                  value={subscriptionQuery}
                  onChange={(e) => setSubscriptionQuery(e.currentTarget.value)}
                />
                <select
                  className="admin-select rounded-xl border border-slate-300 px-3 py-2 bg-transparent"
                  value={subscriptionStatusFilter}
                  onChange={(e) => setSubscriptionStatusFilter(e.currentTarget.value)}
                >
                  <option value="all">All statuses</option>
                  <option value="active">Active</option>
                  <option value="suspended">Suspended</option>
                  <option value="canceled">Canceled</option>
                  <option value="grace">Grace</option>
                </select>
                <select
                  className="admin-select rounded-xl border border-slate-300 px-3 py-2 bg-transparent"
                  value={subscriptionSort}
                  onChange={(e) => setSubscriptionSort(e.currentTarget.value)}
                >
                  <option value="renewal_desc">Sort: Renewal latest</option>
                  <option value="renewal_asc">Sort: Renewal soonest</option>
                  <option value="email_asc">Sort: Email A-Z</option>
                  <option value="email_desc">Sort: Email Z-A</option>
                  <option value="seats_desc">Sort: Seats high-low</option>
                  <option value="seats_asc">Sort: Seats low-high</option>
                </select>
                <div className="flex items-center text-sm" style={{ color: "var(--muted)" }}>
                  Showing {filteredSubscriptions.length.toLocaleString()} subscriptions
                </div>
              </div>
              <div className="mt-3 overflow-x-auto overflow-y-auto" style={{ maxHeight: "68vh" }}>
                <table className="w-full text-sm">
                  <thead>
                    <tr style={{ color: "var(--muted)" }}>
                      <th className="text-left">Email</th><th className="text-left">Subscription</th><th className="text-left">Status</th><th className="text-left">Seats</th><th className="text-left">In Use</th><th className="text-left">Renewal</th><th className="text-left">Action</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filteredSubscriptions.map((s: any) => (
                      <tr key={s.subscriptionId}>
                        <td>{s.email || "unknown"}</td>
                        <td className="font-mono">{shortId(s.subscriptionId)}</td>
                        <td>{s.status}</td>
                        <td>{s.seatLimit}</td>
                        <td>{s.seatInUse ?? 0}</td>
                        <td>{formatUnix(s.currentPeriodEndUnix)}</td>
                        <td>
                          {s.status === "suspended" ? (
                            <button className="btn btn-secondary" onClick={() => void unsuspend(s.subscriptionId)} type="button">Unsuspend</button>
                          ) : (
                            <button className="btn btn-secondary" onClick={() => void suspend(s.subscriptionId)} type="button">Suspend</button>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          ) : null}

          {tab === "devices" ? (
            <section className="mt-4 panel p-4">
              <h3 className="font-semibold">Devices</h3>
              <div className="mt-3 grid gap-2 md:grid-cols-4">
                <input
                  className="rounded-xl border border-slate-300 px-3 py-2"
                  placeholder="Search device, os, country..."
                  value={deviceQuery}
                  onChange={(e) => setDeviceQuery(e.currentTarget.value)}
                />
                <select
                  className="admin-select rounded-xl border border-slate-300 px-3 py-2 bg-transparent"
                  value={deviceStatusFilter}
                  onChange={(e) => setDeviceStatusFilter(e.currentTarget.value)}
                >
                  <option value="all">All statuses</option>
                  <option value="active">Active</option>
                  <option value="retired">Retired</option>
                </select>
                <select
                  className="admin-select rounded-xl border border-slate-300 px-3 py-2 bg-transparent"
                  value={deviceSort}
                  onChange={(e) => setDeviceSort(e.currentTarget.value)}
                >
                  <option value="last_seen_desc">Sort: Last seen newest</option>
                  <option value="last_seen_asc">Sort: Last seen oldest</option>
                  <option value="device_asc">Sort: Device A-Z</option>
                  <option value="device_desc">Sort: Device Z-A</option>
                  <option value="os_asc">Sort: OS A-Z</option>
                  <option value="country_asc">Sort: Country A-Z</option>
                </select>
                <div className="flex items-center text-sm" style={{ color: "var(--muted)" }}>
                  Showing {filteredDevices.length.toLocaleString()} devices
                </div>
              </div>
              <div className="mt-3 overflow-x-auto overflow-y-auto" style={{ maxHeight: "68vh" }}>
                <table className="w-full text-sm">
                  <thead>
                    <tr style={{ color: "var(--muted)" }}>
                      <th className="text-left">Device</th><th className="text-left">OS</th><th className="text-left">Country</th><th className="text-left">Seat</th><th className="text-left">Status</th><th className="text-left">Last Seen</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filteredDevices.map((b: any) => (
                      <tr key={b.bindingId}>
                        <td>{b.deviceName || shortId(b.deviceId)}</td>
                        <td>{b.os || "unknown"}</td>
                        <td>{b.countryCode || "unknown"}</td>
                        <td>{b.seatIndex}</td>
                        <td>{b.status}</td>
                        <td>{formatUnix(b.lastSeenUnix)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          ) : null}

          {tab === "billing" ? (
            <section className="mt-4 grid gap-4 lg:grid-cols-2">
              <div className="panel p-4">
                <h3 className="font-semibold">Revenue</h3>
                <p className="mt-2 text-sm">Contracted MRR: <strong>{moneyCents(totals.contracted_mrr_cents)}</strong></p>
                <p className="text-sm">Collected cash ({period}): <strong>{moneyCents(totals.collected_cash_cents_period)}</strong></p>
                <p className="text-sm">Refunds ({period}): <strong>{moneyCents(totals.refunds_cents_period)}</strong></p>
                <p className="text-sm">Net ({period}): <strong>{moneyCents(totals.net_cents_period)}</strong></p>
              </div>
              <div className="panel p-4" style={{ height: 280 }}>
                <h3 className="font-semibold">Revenue by Time</h3>
                <ResponsiveContainer width="100%" height="90%">
                  <AreaChart data={series.revenue_over_time ?? []}>
                    <CartesianGrid strokeDasharray="3 3" />
                    <XAxis dataKey="ts" tickFormatter={(v) => new Date(v * 1000).toLocaleDateString()} />
                    <YAxis />
                    <Tooltip labelFormatter={(v) => formatUnix(Number(v))} />
                    <Area type="monotone" dataKey="value" stroke="#0ea5e9" fill="#0ea5e933" />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
            </section>
          ) : null}

          {tab === "reports" ? (
            <section className="mt-4 grid gap-4 lg:grid-cols-2">
              <div className="panel p-4">
                <h3 className="font-semibold">Exports</h3>
                <div className="mt-3 flex flex-wrap gap-2">
                  <button className="btn btn-secondary" type="button" onClick={() => exportCsv("customers")}>Customers CSV</button>
                  <button className="btn btn-secondary" type="button" onClick={() => exportCsv("subscriptions")}>Subscriptions CSV</button>
                  <button className="btn btn-secondary" type="button" onClick={() => exportCsv("devices")}>Devices CSV</button>
                  <button className="btn btn-secondary" type="button" onClick={() => exportCsv("revenue")}>Revenue CSV</button>
                </div>
              </div>
              <div className="panel p-4" style={{ height: 280 }}>
                <h3 className="font-semibold">OS Mix</h3>
                <ResponsiveContainer width="100%" height="90%">
                  <PieChart>
                    <Pie data={series.os_distribution ?? []} dataKey="value" nameKey="label" outerRadius={90} fill="#14b8a6" />
                    <Tooltip />
                  </PieChart>
                </ResponsiveContainer>
              </div>
              <div className="panel p-4" style={{ height: 280 }}>
                <h3 className="font-semibold">Country Mix</h3>
                <ResponsiveContainer width="100%" height="90%">
                  <BarChart data={series.country_distribution ?? []}>
                    <CartesianGrid strokeDasharray="3 3" />
                    <XAxis dataKey="label" />
                    <YAxis />
                    <Tooltip />
                    <Bar dataKey="value" fill="#f59e0b" />
                  </BarChart>
                </ResponsiveContainer>
              </div>
            </section>
          ) : null}

          {tab === "health" ? (
            <section className="mt-4 panel p-4">
              <h3 className="font-semibold">System Health</h3>
              <div className="mt-3 grid gap-3 md:grid-cols-2">
                <div className="rounded-xl border p-3" style={{ borderColor: "var(--line)" }}>
                  <p className="text-xs" style={{ color: "var(--muted)" }}>Broker</p>
                  <p className="text-xl font-bold">{state.health?.broker_up ? "Up" : "Down"}</p>
                </div>
                <div className="rounded-xl border p-3" style={{ borderColor: "var(--line)" }}>
                  <p className="text-xs" style={{ color: "var(--muted)" }}>Reminder Queue</p>
                  <p className="text-xl font-bold">{Number(state.health?.reminders_due ?? 0).toLocaleString()}</p>
                </div>
                <div className="rounded-xl border p-3" style={{ borderColor: "var(--line)" }}>
                  <p className="text-xs" style={{ color: "var(--muted)" }}>Reminder Failures (24h)</p>
                  <p className="text-xl font-bold">{Number(state.health?.reminders_failed_24h ?? 0).toLocaleString()}</p>
                </div>
                <div className="rounded-xl border p-3" style={{ borderColor: "var(--line)" }}>
                  <p className="text-xs" style={{ color: "var(--muted)" }}>Updated</p>
                  <p className="text-xl font-bold">{formatUnix(state.health?.timestamp_unix)}</p>
                </div>
              </div>
            </section>
          ) : null}

          {tab === "smtp" ? (
            <section className="mt-4 panel p-4">
              <h3 className="font-semibold">SMTP Settings</h3>
              <p className="mt-1 text-sm" style={{ color: "var(--muted)" }}>Used for customer access and billing reminder emails.</p>
              <div className="mt-3 grid gap-2 md:grid-cols-2">
                <input className="rounded-xl border border-slate-300 px-3 py-2" placeholder="SMTP host" value={smtp.host} onChange={(e) => { const value = e.currentTarget.value; setSmtp((p) => ({ ...p, host: value })); }} />
                <input className="rounded-xl border border-slate-300 px-3 py-2" placeholder="Port" type="number" value={smtp.port} onChange={(e) => { const value = Number(e.currentTarget.value || 0); setSmtp((p) => ({ ...p, port: value })); }} />
                <input className="rounded-xl border border-slate-300 px-3 py-2" placeholder="Username" value={smtp.username} onChange={(e) => { const value = e.currentTarget.value; setSmtp((p) => ({ ...p, username: value })); }} />
                <input className="rounded-xl border border-slate-300 px-3 py-2" placeholder={smtpPasswordSet ? "Password (leave blank to keep current)" : "Password"} type="password" value={smtp.password} onChange={(e) => { const value = e.currentTarget.value; setSmtp((p) => ({ ...p, password: value })); }} />
                <input className="rounded-xl border border-slate-300 px-3 py-2" placeholder="From email" value={smtp.fromEmail} onChange={(e) => { const value = e.currentTarget.value; setSmtp((p) => ({ ...p, fromEmail: value })); }} />
                <input className="rounded-xl border border-slate-300 px-3 py-2" placeholder="From name" value={smtp.fromName} onChange={(e) => { const value = e.currentTarget.value; setSmtp((p) => ({ ...p, fromName: value })); }} />
              </div>
              <label className="mt-3 flex items-center gap-2 text-sm">
                <input type="checkbox" checked={smtp.secure} onChange={(e) => { const checked = e.currentTarget.checked; setSmtp((p) => ({ ...p, secure: checked })); }} />
                Use TLS/SSL (`secure`)
              </label>
              <div className="mt-3 flex flex-wrap gap-2">
                <button className="btn btn-primary" type="button" onClick={saveSmtp}>Save SMTP</button>
              </div>
              <div className="mt-3 flex flex-wrap gap-2">
                <input className="rounded-xl border border-slate-300 px-3 py-2" placeholder="test@company.com" value={smtpTestEmail} onChange={(e) => setSmtpTestEmail(e.currentTarget.value)} />
                <button className="btn btn-secondary" type="button" onClick={sendSmtpTest}>Send Test Email</button>
              </div>
            </section>
          ) : null}
        </>
      )}
    </main>
  );
}
