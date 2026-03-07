"use client";

import { useMemo, useState } from "react";
import {
  MAX_SELF_SERVE_SEATS,
  monthlyTotalForSeats,
  rateForSeats
} from "@/lib/pricing";

export function SeatPricingCard() {
  const [seats, setSeats] = useState<number>(1);
  const [email, setEmail] = useState<string>("");

  const rate = useMemo(() => rateForSeats(seats), [seats]);
  const total = useMemo(() => monthlyTotalForSeats(seats), [seats]);

  return (
    <div id="pricing" className="panel p-5">
      <h2 className="text-2xl font-semibold">Start Your Team Plan</h2>
      <p className="mt-2 text-sm" style={{ color: "var(--muted)" }}>
        Pick your team size and launch checkout in seconds.
      </p>

      <div className="mt-5">
        <label className="text-sm font-semibold">Seats: {seats}</label>
        <input
          className="mt-2 w-full"
          type="range"
          min={1}
          max={MAX_SELF_SERVE_SEATS}
          step={1}
          value={seats}
          onChange={(e) => setSeats(Number(e.currentTarget.value))}
        />
      </div>

      <div className="mt-5 rounded-xl border p-4" style={{ borderColor: "var(--line)", background: "var(--soft)" }}>
        <div className="text-sm" style={{ color: "var(--muted)" }}>Effective rate</div>
        <div className="text-xl font-semibold">${rate}/seat</div>
        <div className="mt-2 text-sm" style={{ color: "var(--muted)" }}>Estimated monthly total</div>
        <div className="text-3xl font-bold">${total}/month</div>
      </div>

      <form className="mt-5 grid gap-2" action="/checkout/start" method="post">
        <input
          className="rounded-xl border px-3 py-2"
          style={{ borderColor: "var(--line)", background: "var(--bg)" }}
          placeholder="you@company.com"
          value={email}
          onChange={(e) => setEmail(e.currentTarget.value)}
          type="email"
          name="email"
          required
        />
        <input type="hidden" name="seats" value={seats} />
        <button
          className="btn btn-primary"
          disabled={!email}
          type="submit"
        >
          Checkout with Stripe
        </button>
        <a className="btn btn-secondary text-center" href="/contact">
          Need more than 100 seats? Contact Sales
        </a>
      </form>
    </div>
  );
}
