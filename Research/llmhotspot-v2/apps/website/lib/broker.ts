import { getEnv } from "@/lib/env";

type BrokerOptions = {
  method?: string;
  body?: unknown;
  headers?: Record<string, string>;
  noJson?: boolean;
};

export async function brokerRequest(path: string, options: BrokerOptions = {}): Promise<Response> {
  const env = getEnv();
  const url = `${env.WEBSITE_BROKER_BASE_URL.replace(/\/$/, "")}${path}`;
  const headers: Record<string, string> = {
    accept: "application/json",
    ...(options.noJson ? {} : { "content-type": "application/json" }),
    ...(options.headers ?? {})
  };
  if (env.WEBSITE_BROKER_SHARED_SECRET) {
    headers["x-broker-secret"] = env.WEBSITE_BROKER_SHARED_SECRET;
  }
  const init: RequestInit = {
    method: options.method ?? "POST",
    headers
  };
  if (options.body !== undefined) {
    init.body = options.noJson ? (options.body as string) : JSON.stringify(options.body);
  }
  return fetch(url, init);
}

export async function brokerJson<T = any>(path: string, options: BrokerOptions = {}): Promise<T> {
  const resp = await brokerRequest(path, options);
  const data = await resp.json().catch(() => ({ ok: false, error: "invalid_json" }));
  if (!resp.ok) {
    const err = new Error(data?.error ?? `broker_request_failed_${resp.status}`);
    (err as Error & { status?: number; payload?: unknown }).status = resp.status;
    (err as Error & { status?: number; payload?: unknown }).payload = data;
    throw err;
  }
  return data as T;
}
