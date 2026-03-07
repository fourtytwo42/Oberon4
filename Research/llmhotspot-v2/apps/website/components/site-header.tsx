"use client";

import Image from "next/image";
import { useMemo, useState } from "react";
import type { ThemeMode } from "@/components/theme-provider";

type SiteHeaderProps = {
  theme: ThemeMode;
  setTheme: (next: ThemeMode | ((prev: ThemeMode) => ThemeMode)) => void;
  links?: Array<{ label: string; href: string }>;
};

export function SiteHeader({ theme, setTheme, links = [] }: SiteHeaderProps) {
  const [logoHover, setLogoHover] = useState(false);
  const logoSrc = useMemo(() => {
    if (theme === "dark") {
      return logoHover ? "/logo-hover-white.png" : "/light-logo.png";
    }
    return logoHover ? "/logo-hover-black.png" : "/dark-logo.png";
  }, [theme, logoHover]);

  return (
    <header className="mb-8 flex flex-wrap items-center justify-between gap-3">
      <a href="/" className="inline-flex items-center gap-3 text-xl font-bold tracking-tight">
        <Image
          src={logoSrc}
          alt="LLM Hotspot logo"
          width={42}
          height={42}
          className="brand-logo"
          onMouseEnter={() => setLogoHover(true)}
          onMouseLeave={() => setLogoHover(false)}
        />
        <span>LLM Hotspot</span>
      </a>
      <div className="flex items-center gap-3">
        {links.length > 0 ? (
          <nav className="flex flex-wrap items-center gap-4 text-sm" style={{ color: "var(--muted)" }}>
            {links.map((l) => (
              <a key={`${l.label}-${l.href}`} href={l.href}>{l.label}</a>
            ))}
          </nav>
        ) : null}
        <button
          className="theme-toggle"
          type="button"
          aria-label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
          title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
          onClick={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        >
          {theme === "dark" ? (
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M12 3v2m0 14v2m9-9h-2M5 12H3m15.36 6.36-1.42-1.42M7.05 7.05 5.64 5.64m12.72 0-1.42 1.41M7.05 16.95l-1.41 1.41M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          ) : (
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                d="M21 14.5A8.5 8.5 0 1 1 9.5 3 7 7 0 0 0 21 14.5Z"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          )}
        </button>
      </div>
    </header>
  );
}
