import { z } from "zod";

const schema = z.object({
  WEBSITE_BROKER_BASE_URL: z.string().url(),
  WEBSITE_BROKER_SHARED_SECRET: z.string().min(1).optional(),
  WEBSITE_POSTGRES_URL: z.string().min(1),
  WEBSITE_REDIS_URL: z.string().min(1).optional(),
  WEBSITE_PUBLIC_BASE_URL: z.string().url().default("http://127.0.0.1:3000"),
  WEBSITE_CHECKOUT_SUCCESS_URL: z.string().url(),
  WEBSITE_CHECKOUT_CANCEL_URL: z.string().url(),
  WEBSITE_CONTACT_SALES_URL: z.string().url().default("https://llmhotspot.com/contact"),
  WEBSITE_CUSTOMER_SESSION_TTL_SECS: z.coerce.number().int().positive().default(8 * 60 * 60),
  WEBSITE_ADMIN_SESSION_TTL_SECS: z.coerce.number().int().positive().default(8 * 60 * 60),
  WEBSITE_REMINDER_INTERNAL_SECRET: z.string().min(1).optional()
});

export type WebsiteEnv = z.infer<typeof schema>;

let cached: WebsiteEnv | null = null;

export function getEnv(): WebsiteEnv {
  if (cached) {
    return cached;
  }
  cached = schema.parse(process.env);
  return cached;
}
