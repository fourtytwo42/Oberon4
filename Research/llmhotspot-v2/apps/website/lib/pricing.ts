export type PricingTier = {
  min: number;
  max: number;
  rate: number;
};

export const MAX_SELF_SERVE_SEATS = 100;

export const PRICING_TIERS: PricingTier[] = [
  { min: 1, max: 1, rate: 20 },
  { min: 2, max: 5, rate: 18 },
  { min: 6, max: 10, rate: 17 },
  { min: 11, max: 20, rate: 16 },
  { min: 21, max: 30, rate: 14 },
  { min: 31, max: 50, rate: 12 },
  { min: 51, max: 100, rate: 10 }
];

export const SEAT_SNAP_POINTS = [1, 2, 6, 11, 21, 31, 51, 100] as const;

export function rateForSeats(seats: number): number {
  const bounded = Math.max(1, Math.min(MAX_SELF_SERVE_SEATS, seats));
  const tier = PRICING_TIERS.find((t) => bounded >= t.min && bounded <= t.max);
  return tier ? tier.rate : PRICING_TIERS[PRICING_TIERS.length - 1].rate;
}

export function monthlyTotalForSeats(seats: number): number {
  const bounded = Math.max(1, Math.min(MAX_SELF_SERVE_SEATS, seats));
  return bounded * rateForSeats(bounded);
}
