import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

const ADMIN_PORT = "3010";

function extractPort(request: NextRequest): string {
  const forwardedPort = request.headers.get("x-forwarded-port");
  if (forwardedPort && forwardedPort.trim().length > 0) {
    return forwardedPort.trim();
  }

  const forwardedHost = request.headers.get("x-forwarded-host");
  if (forwardedHost) {
    const maybePort = forwardedHost.split(",")[0]?.trim().split(":")[1];
    if (maybePort) {
      return maybePort;
    }
  }

  const host = request.headers.get("host");
  if (host) {
    const maybePort = host.split(":")[1];
    if (maybePort) {
      return maybePort;
    }
  }

  if (request.nextUrl.port) {
    return request.nextUrl.port;
  }

  return "";
}

export function middleware(request: NextRequest): NextResponse {
  const pathname = request.nextUrl.pathname;
  const isAdminPath = pathname === "/admin" || pathname.startsWith("/admin/");
  const isAdminApiPath = pathname === "/api/admin" || pathname.startsWith("/api/admin/");
  if (!isAdminPath && !isAdminApiPath) {
    return NextResponse.next();
  }

  const requestPort = extractPort(request);
  if (requestPort === ADMIN_PORT) {
    return NextResponse.next();
  }

  if (isAdminApiPath) {
    return NextResponse.json({ ok: false, error: "admin_port_only" }, { status: 403 });
  }

  return new NextResponse("Not Found", { status: 404 });
}

export const config = {
  matcher: ["/admin/:path*", "/api/admin/:path*"],
};
