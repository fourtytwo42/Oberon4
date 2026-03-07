import "@/app/globals.css";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "LLM Hotspot",
  description: "Seat-based LLM tunnel access with billing and admin controls"
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
