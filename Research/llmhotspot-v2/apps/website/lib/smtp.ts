import nodemailer from "nodemailer";
import { ensureWebsiteSchema, getDb } from "@/lib/db";
import { getEnv } from "@/lib/env";

export type SmtpSettings = {
  host: string;
  port: number;
  secure: boolean;
  username: string;
  password: string;
  fromEmail: string;
  fromName: string | null;
};

function nowUnix(): number {
  return Math.floor(Date.now() / 1000);
}

const CONFIG_ID = "default";

export async function getSmtpSettings(): Promise<SmtpSettings | null> {
  await ensureWebsiteSchema();
  const db = getDb();
  const row = await db.query(
    `SELECT host, port, secure, username, password, from_email, from_name
     FROM web_smtp_settings WHERE config_id = $1 LIMIT 1`,
    [CONFIG_ID]
  );
  if (row.rows.length === 0) {
    return null;
  }
  return {
    host: row.rows[0].host,
    port: Number(row.rows[0].port),
    secure: Boolean(row.rows[0].secure),
    username: row.rows[0].username,
    password: row.rows[0].password,
    fromEmail: row.rows[0].from_email,
    fromName: row.rows[0].from_name ?? null
  };
}

export async function upsertSmtpSettings(settings: SmtpSettings): Promise<void> {
  await ensureWebsiteSchema();
  const db = getDb();
  await db.query(
    `INSERT INTO web_smtp_settings(
        config_id, host, port, secure, username, password, from_email, from_name, updated_unix
      )
      VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
      ON CONFLICT (config_id) DO UPDATE SET
        host = EXCLUDED.host,
        port = EXCLUDED.port,
        secure = EXCLUDED.secure,
        username = EXCLUDED.username,
        password = EXCLUDED.password,
        from_email = EXCLUDED.from_email,
        from_name = EXCLUDED.from_name,
        updated_unix = EXCLUDED.updated_unix`,
    [
      CONFIG_ID,
      settings.host,
      settings.port,
      settings.secure,
      settings.username,
      settings.password,
      settings.fromEmail,
      settings.fromName,
      nowUnix()
    ]
  );
}

export function customerManageAccessUrl(accessToken: string): string {
  const env = getEnv();
  return `${env.WEBSITE_PUBLIC_BASE_URL.replace(/\/$/, "")}/manage?access=${encodeURIComponent(accessToken)}`;
}

export async function sendCustomerAccessEmail(
  email: string,
  accessToken: string,
  subjectPrefix = "Your LLM Hotspot access link"
): Promise<void> {
  const settings = await getSmtpSettings();
  if (!settings) {
    throw new Error("smtp_not_configured");
  }
  const url = customerManageAccessUrl(accessToken);
  const from = settings.fromName
    ? `"${settings.fromName}" <${settings.fromEmail}>`
    : settings.fromEmail;
  await sendMailWithSettings(settings, {
    from,
    to: email,
    subject: subjectPrefix,
    text: `Open your seat management page: ${url}`,
    html: `<p>Open your seat management page:</p><p><a href="${url}">${url}</a></p><p>This link does not expire unless revoked by an admin.</p>`
  });
}

async function sendMailWithSettings(
  settings: SmtpSettings,
  mail: {
    from: string;
    to: string;
    subject: string;
    text: string;
    html: string;
  }
): Promise<void> {
  const transporter = nodemailer.createTransport({
    host: settings.host,
    port: settings.port,
    secure: settings.secure,
    auth: {
      user: settings.username,
      pass: settings.password
    }
  });

  await transporter.sendMail(mail);
}

export async function sendSmtpTestEmail(
  settings: SmtpSettings,
  toEmail: string
): Promise<void> {
  const from = settings.fromName
    ? `"${settings.fromName}" <${settings.fromEmail}>`
    : settings.fromEmail;
  await sendMailWithSettings(settings, {
    from,
    to: toEmail,
    subject: "LLM Hotspot SMTP test",
    text: "SMTP test successful. Your LLM Hotspot website can send email.",
    html: "<p>SMTP test successful.</p><p>Your LLM Hotspot website can send email.</p>"
  });
}

export type BillingReminderMail = {
  reminderId: string;
  email: string;
  subscriptionId: string;
  licenseId: string | null;
  reason: string;
  dueUnix: number;
  graceExpiresUnix: number | null;
  currentPeriodEndUnix: number | null;
  seats: number;
  status: string;
};

function shortId(input: string | null | undefined): string {
  if (!input) {
    return "-";
  }
  if (input.length <= 14) {
    return input;
  }
  return `${input.slice(0, 10)}...${input.slice(-4)}`;
}

function fmtDate(unixTs: number | null | undefined): string {
  if (!unixTs || unixTs <= 0) {
    return "-";
  }
  return new Date(unixTs * 1000).toLocaleString("en-US", { timeZone: "UTC", timeZoneName: "short" });
}

function reminderMessage(reason: string): { subject: string; lead: string } {
  switch (reason) {
    case "payment_failed_immediate":
      return {
        subject: "Payment failed: grace period started",
        lead: "We could not process your latest subscription payment. Your grace period has started."
      };
    case "payment_failed_7d":
      return {
        subject: "Action needed: access expires in 7 days",
        lead: "Your payment has not succeeded yet. Access expires in 7 days unless billing is fixed."
      };
    case "payment_failed_3d":
      return {
        subject: "Action needed: access expires in 3 days",
        lead: "Your payment is still failing. Access expires in 3 days unless billing is fixed."
      };
    case "payment_failed_1d":
      return {
        subject: "Final notice: access expires in 1 day",
        lead: "Your payment is still failing. Access expires in 1 day unless billing is fixed."
      };
    case "cancel_immediate":
      return {
        subject: "Subscription canceled: access remains until end date",
        lead: "Your subscription was canceled. Access remains available until the end of your paid period."
      };
    case "cancel_7d":
      return {
        subject: "Subscription ending in 7 days",
        lead: "Your canceled subscription is scheduled to end in 7 days."
      };
    case "cancel_3d":
      return {
        subject: "Subscription ending in 3 days",
        lead: "Your canceled subscription is scheduled to end in 3 days."
      };
    case "cancel_1d":
      return {
        subject: "Subscription ending in 1 day",
        lead: "Your canceled subscription is scheduled to end in 1 day."
      };
    default:
      return {
        subject: "Billing notice",
        lead: "There is an update regarding your subscription billing status."
      };
  }
}

export async function sendBillingReminderEmail(reminder: BillingReminderMail): Promise<void> {
  const settings = await getSmtpSettings();
  if (!settings) {
    throw new Error("smtp_not_configured");
  }
  const env = getEnv();
  const from = settings.fromName
    ? `"${settings.fromName}" <${settings.fromEmail}>`
    : settings.fromEmail;
  const manageUrl = `${env.WEBSITE_PUBLIC_BASE_URL.replace(/\/$/, "")}/manage`;
  const msg = reminderMessage(reminder.reason);
  const graceEnds = fmtDate(reminder.graceExpiresUnix);
  const periodEnds = fmtDate(reminder.currentPeriodEndUnix);
  const subscriptionLabel = shortId(reminder.subscriptionId);
  const licenseLabel = shortId(reminder.licenseId);
  await sendMailWithSettings(settings, {
    from,
    to: reminder.email,
    subject: `LLM Hotspot: ${msg.subject}`,
    text: `${msg.lead}

Subscription: ${subscriptionLabel}
License: ${licenseLabel}
Seats: ${reminder.seats}
Status: ${reminder.status}
Grace ends: ${graceEnds}
Current period end: ${periodEnds}

Manage your subscription: ${manageUrl}
If needed, open Billing Portal from the management page.`,
    html: `
      <p>${msg.lead}</p>
      <p><strong>Subscription:</strong> ${subscriptionLabel}<br/>
      <strong>License:</strong> ${licenseLabel}<br/>
      <strong>Seats:</strong> ${reminder.seats}<br/>
      <strong>Status:</strong> ${reminder.status}<br/>
      <strong>Grace ends:</strong> ${graceEnds}<br/>
      <strong>Current period end:</strong> ${periodEnds}</p>
      <p><a href="${manageUrl}">Open Customer Seat Management</a></p>
      <p>If needed, open Billing Portal from the management page.</p>
    `
  });
}
