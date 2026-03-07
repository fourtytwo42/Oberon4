import crypto from "node:crypto";
import { ensureWebsiteSchema, getDb } from "@/lib/db";

function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

function token(): string {
  return `wca_${crypto.randomUUID().replace(/-/g, "")}`;
}

export async function issueOneTimeAccessToken(emailRaw: string): Promise<string> {
  await ensureWebsiteSchema();
  const db = getDb();
  const email = emailRaw.trim().toLowerCase();
  const accessToken = token();
  const now = nowUnix();
  await db.query(
    `UPDATE web_customer_access_tokens
     SET revoked_unix = $2
     WHERE email = $1 AND revoked_unix IS NULL AND consumed_unix IS NULL`,
    [email, now]
  );
  await db.query(
    `INSERT INTO web_customer_access_tokens(access_token, email, created_unix, last_used_unix, consumed_unix, revoked_unix)
     VALUES ($1, $2, $3, NULL, NULL, NULL)
     ON CONFLICT (email) DO UPDATE SET
       access_token = EXCLUDED.access_token,
       created_unix = EXCLUDED.created_unix,
       last_used_unix = NULL,
       consumed_unix = NULL,
       revoked_unix = NULL`,
    [accessToken, email, now]
  );
  return accessToken;
}

export async function consumeAccessToken(accessToken: string): Promise<string | null> {
  await ensureWebsiteSchema();
  const db = getDb();
  const row = await db.query(
    `UPDATE web_customer_access_tokens
     SET consumed_unix = $2, last_used_unix = $2
     WHERE access_token = $1
       AND revoked_unix IS NULL
       AND consumed_unix IS NULL
     RETURNING email`,
    [accessToken, nowUnix()]
  );
  if (row.rows.length === 0) {
    return null;
  }
  return row.rows[0].email as string;
}
