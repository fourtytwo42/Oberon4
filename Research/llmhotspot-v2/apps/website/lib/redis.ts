import Redis from "ioredis";
import { getEnv } from "@/lib/env";

let redis: Redis | null = null;

export function getRedis(): Redis | null {
  if (redis) {
    return redis;
  }
  const env = getEnv();
  if (!env.WEBSITE_REDIS_URL) {
    return null;
  }
  redis = new Redis(env.WEBSITE_REDIS_URL, { maxRetriesPerRequest: 1 });
  return redis;
}

export async function cacheGet(key: string): Promise<string | null> {
  const r = getRedis();
  if (!r) {
    return null;
  }
  try {
    return await r.get(key);
  } catch {
    return null;
  }
}

export async function cacheSet(key: string, value: string, ttlSecs: number): Promise<void> {
  const r = getRedis();
  if (!r) {
    return;
  }
  try {
    await r.set(key, value, "EX", ttlSecs);
  } catch {
    // Fail open when cache is unavailable.
  }
}

export async function incrementRateLimit(
  key: string,
  windowSecs: number
): Promise<number> {
  const r = getRedis();
  if (!r) {
    return 1;
  }
  try {
    const count = await r.incr(key);
    if (count === 1) {
      await r.expire(key, windowSecs);
    }
    return count;
  } catch {
    return 1;
  }
}
