const DATABASE_URL = process.env.DATABASE_URL;
const SLACK_WEBHOOK_URL = process.env.SLACK_MEET_WEBHOOK_URL;
const { spawnSync } = require("child_process");

if (!DATABASE_URL) {
  console.error("Missing DATABASE_URL. Exiting.");
  process.exit(1);
}

if (!SLACK_WEBHOOK_URL) {
  console.error("Missing SLACK_MEET_WEBHOOK_URL. Exiting.");
  process.exit(1);
}

function toDateKey(date) {
  return date.toISOString().slice(0, 10);
}

function addMonths(date, months) {
  const next = new Date(date);
  next.setUTCMonth(next.getUTCMonth() + months);
  return next;
}

function formatMeetDate(meet) {
  if (!meet.endDate || meet.endDate === meet.startDate) {
    return meet.startDate;
  }

  return `${meet.startDate} to ${meet.endDate}`;
}

function queryPostgresMeets(todayKey, cutoffKey) {
  const sql = `
    SELECT name, start_date::text, end_date::text
    FROM meets
    WHERE status <> 'completed'
      AND start_date >= '${todayKey}'::date
      AND start_date <= '${cutoffKey}'::date
    ORDER BY start_date, name
  `;
  const result = spawnSync("psql", [DATABASE_URL, "-X", "-q", "-t", "-A", "-F", "\t", "-c", sql], {
    encoding: "utf8",
  });

  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `Postgres query failed with exit code ${result.status}`);
  }

  return result.stdout
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => {
      const [name, startDate, endDate] = line.split("\t");
      return { name, startDate, endDate };
    });
}

async function sendSlackMessage(text) {
  const response = await fetch(SLACK_WEBHOOK_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });

  if (!response.ok) {
    throw new Error(`Slack webhook failed: ${response.status} ${await response.text()}`);
  }
}

async function main() {
  const today = new Date();
  const todayKey = toDateKey(today);
  const cutoffKey = toDateKey(addMonths(today, 2));

  const upcomingMeets = queryPostgresMeets(todayKey, cutoffKey);
  if (!Array.isArray(upcomingMeets)) {
    throw new Error("Meet query did not return a meets array.");
  }

  const header = `Upcoming meets in Postgres from ${todayKey} through ${cutoffKey}`;
  const message =
    upcomingMeets.length === 0
      ? `${header}\n\nNo meets found.`
      : `${header}\n\n${upcomingMeets
          .map((meet) => `- ${meet.name}: ${formatMeetDate(meet)}`)
          .join("\n")}`;

  await sendSlackMessage(message);
  console.log(`Sent ${upcomingMeets.length} upcoming meet(s) to Slack.`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
