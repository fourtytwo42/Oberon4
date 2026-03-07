const RELEASES_URL = "https://github.com/fourtytwo42/llmhotspot-v2/releases/latest";

export default function DownloadPage() {
  return (
    <main className="shell">
      <section className="panel p-6">
        <p className="text-sm font-semibold uppercase tracking-wider" style={{ color: "var(--brand)" }}>
          LLM Hotspot Desktop
        </p>
        <h1 className="mt-2 text-4xl font-black leading-tight">Download the free app and get started in minutes</h1>
        <p className="mt-3 max-w-3xl text-sm" style={{ color: "var(--muted)" }}>
          Free mode is available now. Install the desktop app, connect your accounts, and run a unified endpoint for your tools. Upgrade later when you need managed team seats and production operations.
        </p>

        <div className="mt-5 grid gap-3 md:grid-cols-3">
          <a className="btn btn-primary text-center" href={RELEASES_URL} target="_blank" rel="noreferrer">
            Download for macOS
          </a>
          <a className="btn btn-primary text-center" href={RELEASES_URL} target="_blank" rel="noreferrer">
            Download for Windows
          </a>
          <a className="btn btn-primary text-center" href={RELEASES_URL} target="_blank" rel="noreferrer">
            Download for Linux
          </a>
        </div>

        <div className="mt-6 rounded-xl border p-4 text-sm" style={{ borderColor: "var(--line)", color: "var(--muted)" }}>
          All download buttons open the latest release page where platform installers and binaries are published.
        </div>

        <div className="mt-5 flex flex-wrap gap-3">
          <a className="btn btn-secondary" href="/">Back to Homepage</a>
          <a className="btn btn-secondary" href="/manage">Already a customer?</a>
        </div>
      </section>
    </main>
  );
}
