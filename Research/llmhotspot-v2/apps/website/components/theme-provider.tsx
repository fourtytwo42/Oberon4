"use client";

import { useEffect, useState } from "react";

export type ThemeMode = "light" | "dark";

const STORAGE_KEY = "llmhotspot.website.theme";

function preferredTheme(): ThemeMode {
  if (typeof window === "undefined") {
    return "dark";
  }
  const fromStorage = window.localStorage.getItem(STORAGE_KEY);
  if (fromStorage === "light" || fromStorage === "dark") {
    return fromStorage;
  }
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function useSiteTheme() {
  const [theme, setTheme] = useState<ThemeMode>("dark");

  useEffect(() => {
    const mode = preferredTheme();
    setTheme(mode);
    document.documentElement.setAttribute("data-theme", mode);
  }, []);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(STORAGE_KEY, theme);
    }
  }, [theme]);

  return { theme, setTheme };
}
