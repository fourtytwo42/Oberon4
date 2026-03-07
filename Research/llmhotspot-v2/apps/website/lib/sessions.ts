import crypto from "node:crypto";
import { cookies } from "next/headers";
import { ensureWebsiteSchema, getDb } from "@/lib/db";
import { getEnv } from "@/lib/env";

const CUSTOMER_COOKIE = "llmhotspot_web_customer";
const ADMIN_COOKIE = "llmhotspot_web_admin";
const CUSTOMER_COOKIE_MAX_AGE_SECS = 10 * 365 * 24 * 60 * 60;

function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

function token(prefix: string): string {
  return `${prefix}_${crypto.randomUUID().replace(/-/g, "")}`;
}

export async function createCustomerSession(email: string, contextJson?: string): Promise<string> {
  await ensureWebsiteSchema();
  const db = getDb();
  const sessionId = token("wc");
  const now = nowUnix();
  const expires = now + CUSTOMER_COOKIE_MAX_AGE_SECS;
  await db.query(
    `INSERT INTO web_customer_sessions(session_id, email, challenge_id, created_unix, expires_unix, last_context_json)
     VALUES ($1, $2, NULL, $3, $4, $5)`,
    [sessionId, email.toLowerCase(), now, expires, contextJson ?? null]
  );
  return sessionId;
}

export async function getCustomerSession(): Promise<{ sessionId: string; email: string; contextJson: string | null } | null> {
  await ensureWebsiteSchema();
  const cookieStore = await cookies();
  const sessionId = cookieStore.get(CUSTOMER_COOKIE)?.value;
  if (!sessionId) {
    return null;
  }
  const db = getDb();
  const now = nowUnix();
  const row = await db.query(
    `SELECT session_id, email, last_context_json FROM web_customer_sessions
     WHERE session_id = $1 AND expires_unix >= $2 LIMIT 1`,
    [sessionId, now]
  );
  if (row.rows.length === 0) {
    return null;
  }
  return {
    sessionId: row.rows[0].session_id,
    email: row.rows[0].email,
    contextJson: row.rows[0].last_context_json
  };
}

export async function updateCustomerSessionContext(sessionId: string, contextJson: string): Promise<void> {
  await ensureWebsiteSchema();
  const db = getDb();
  await db.query(`UPDATE web_customer_sessions SET last_context_json = $2 WHERE session_id = $1`, [
    sessionId,
    contextJson
  ]);
}

export async function clearCustomerSession(sessionId: string): Promise<void> {
  await ensureWebsiteSchema();
  const db = getDb();
  await db.query(`DELETE FROM web_customer_sessions WHERE session_id = $1`, [sessionId]);
}

export async function setCustomerCookie(sessionId: string): Promise<void> {
  const cookieStore = await cookies();
  cookieStore.set(CUSTOMER_COOKIE, sessionId, {
    httpOnly: true,
    sameSite: "lax",
    secure: true,
    path: "/",
    maxAge: CUSTOMER_COOKIE_MAX_AGE_SECS
  });
}

export async function clearCustomerCookie(): Promise<void> {
  const cookieStore = await cookies();
  cookieStore.set(CUSTOMER_COOKIE, "", {
    httpOnly: true,
    sameSite: "lax",
    secure: true,
    path: "/",
    maxAge: 0
  });
}

export async function createAdminSession(brokerSessionId: string): Promise<string> {
  await ensureWebsiteSchema();
  const db = getDb();
  const env = getEnv();
  const sessionId = token("wa");
  const now = nowUnix();
  const expires = now + env.WEBSITE_ADMIN_SESSION_TTL_SECS;
  await db.query(
    `INSERT INTO web_admin_sessions(session_id, broker_session_id, created_unix, expires_unix)
     VALUES ($1, $2, $3, $4)`,
    [sessionId, brokerSessionId, now, expires]
  );
  return sessionId;
}

export async function getAdminSession(): Promise<{ sessionId: string; brokerSessionId: string } | null> {
  await ensureWebsiteSchema();
  const cookieStore = await cookies();
  const sessionId = cookieStore.get(ADMIN_COOKIE)?.value;
  if (!sessionId) {
    return null;
  }
  const db = getDb();
  const now = nowUnix();
  const row = await db.query(
    `SELECT session_id, broker_session_id FROM web_admin_sessions
     WHERE session_id = $1 AND expires_unix >= $2 LIMIT 1`,
    [sessionId, now]
  );
  if (row.rows.length === 0) {
    return null;
  }
  return {
    sessionId: row.rows[0].session_id,
    brokerSessionId: row.rows[0].broker_session_id
  };
}

export async function clearAdminSession(sessionId: string): Promise<void> {
  await ensureWebsiteSchema();
  const db = getDb();
  await db.query(`DELETE FROM web_admin_sessions WHERE session_id = $1`, [sessionId]);
}

export async function setAdminCookie(sessionId: string): Promise<void> {
  const env = getEnv();
  const cookieStore = await cookies();
  cookieStore.set(ADMIN_COOKIE, sessionId, {
    httpOnly: true,
    sameSite: "lax",
    // Admin UI is intentionally served on LAN port 3010 and may run on plain HTTP.
    // Secure cookies are ignored on HTTP, causing immediate session loss.
    secure: false,
    path: "/",
    maxAge: env.WEBSITE_ADMIN_SESSION_TTL_SECS
  });
}

export async function clearAdminCookie(): Promise<void> {
  const cookieStore = await cookies();
  cookieStore.set(ADMIN_COOKIE, "", {
    httpOnly: true,
    sameSite: "lax",
    secure: false,
    path: "/",
    maxAge: 0
  });
}
