import { NextRequest } from "next/server";
import { cacheGet, cacheSet } from "@/lib/redis";
import { fail, ok } from "@/lib/http";
import {
  MAX_SELF_SERVE_SEATS,
  monthlyTotalForSeats,
  PRICING_TIERS,
  rateForSeats,
  SEAT_SNAP_POINTS
} from "@/lib/pricing";

const CACHE_KEY = "website:pricing:v1";

export async function GET(_req: NextRequest) {
  const cached = await cacheGet(CACHE_KEY);
  if (cached) {
    return ok(JSON.parse(cached));
  }
  const payload = {
    maxSeats: MAX_SELF_SERVE_SEATS,
    tiers: PRICING_TIERS,
    snapPoints: SEAT_SNAP_POINTS,
    examples: [1, 5, 10, 20, 30, 50, 100].map((seats) => ({
      seats,
      rate: rateForSeats(seats),
      monthlyTotal: monthlyTotalForSeats(seats)
    }))
  };
  try {
    await cacheSet(CACHE_KEY, JSON.stringify(payload), 120);
  } catch {
    // Cache write failures should not fail pricing reads.
  }
  return ok(payload);
}

export async function POST() {
  return fail("method_not_allowed", 405);
}
