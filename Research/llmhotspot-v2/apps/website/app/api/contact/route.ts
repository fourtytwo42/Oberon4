import crypto from "node:crypto";
import { NextRequest } from "next/server";
import { z } from "zod";
import { ensureWebsiteSchema, getDb } from "@/lib/db";
import { fail, ok } from "@/lib/http";

const schema = z.object({
  email: z.string().email(),
  company: z.string().optional(),
  seats: z.number().int().min(101),
  note: z.string().max(4000).optional()
});

function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

export async function POST(req: NextRequest) {
  let bodyRaw: unknown;
  try {
    bodyRaw = await req.json();
  } catch {
    return fail("invalid_json", 400);
  }
  const parsed = schema.safeParse(bodyRaw);
  if (!parsed.success) {
    return fail("invalid_request", 400);
  }
  await ensureWebsiteSchema();
  const db = getDb();
  const requestId = `contact_${crypto.randomUUID().replace(/-/g, "")}`;
  await db.query(
    `INSERT INTO web_contact_requests(request_id, email, company, requested_seats, note, created_unix, status)
     VALUES ($1,$2,$3,$4,$5,$6,$7)`,
    [
      requestId,
      parsed.data.email.toLowerCase(),
      parsed.data.company ?? null,
      parsed.data.seats,
      parsed.data.note ?? null,
      nowUnix(),
      "new"
    ]
  );
  return ok({ requestId });
}
