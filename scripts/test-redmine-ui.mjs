// Browser check against a local read-only public fixture; no real credentials.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir } from "node:fs/promises";
import { createServer } from "node:http";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("..", import.meta.url));
const cache = join(homedir(), ".cache/tmp/opencode");
await mkdir(cache, { recursive: true });
const temp = await mkdtemp(join(cache, "kanso-redmine-browser-"));
const fixture = createServer((request, response) => {
  if (request.headers["x-redmine-api-key"] !== "public-test-key") { response.writeHead(401).end(); return; }
  const url = new URL(request.url, "http://fixture.invalid");
  let data;
  if (url.pathname === "/redmine/users/current.json") {
    data = { user: { id: 9, login: "fixture", firstname: "Morgan", lastname: "Lee" } };
  } else if (url.pathname === "/redmine/projects.json") {
    data = { projects: [{ id: 4, name: "Atlas Platform" }, { id: 5, name: "Beacon Operations" }], total_count: 2, offset: 0, limit: 100 };
  } else if (url.pathname === "/redmine/issues.json") {
    data = { issues: [
      { id: 42, subject: "Investigate API response latency", description: "Review the latest response-time measurements.", project: { id: 4 }, assigned_to: { id: 9 }, status: { is_closed: false } },
      { id: 43, subject: "Document deployment checklist", description: "Capture the steps for the next release.", project: { id: 4 }, assigned_to: { id: 9 }, status: { is_closed: false } }
    ], total_count: 2, offset: 0, limit: 100 };
  } else { response.writeHead(404).end(); return; }
  response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(data));
});
await new Promise((resolve) => fixture.listen(0, "127.0.0.1", resolve));
const fixtureUrl = `http://127.0.0.1:${fixture.address().port}/redmine`;
const probe = createServer();
await new Promise((resolve) => probe.listen(0, "127.0.0.1", resolve));
const port = probe.address().port;
await new Promise((resolve) => probe.close(resolve));
const base = `http://127.0.0.1:${port}`;
const password = "disposable-redmine-browser-test";
const server = spawn(join(root, "target/debug/kanso"), ["serve"], {
  cwd: root,
  env: { ...process.env, KANSO_BIND: `127.0.0.1:${port}`, KANSO_PUBLIC_URL: base,
    KANSO_DATA_DIR: join(temp, "data"), KANSO_PASSWORD: password,
    KANSO_OPENCHAMBER_URL: "", KANSO_OPENCHAMBER_TOKEN: "" },
  stdio: ["ignore", "pipe", "pipe"]
});
let browser;
try {
  await new Promise((resolve, reject) => {
    let output = "";
    const timer = setTimeout(() => reject(new Error("Kanso did not start")), 20000);
    const read = (chunk) => { output += chunk; if (output.includes("kanso listening")) { clearTimeout(timer); resolve(); } };
    server.stdout.on("data", read); server.stderr.on("data", read);
    server.once("exit", () => { clearTimeout(timer); reject(new Error("Kanso exited")); });
  });
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(base);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "Imports", exact: true }).waitFor();
  const projectResponse = await page.request.post(`${base}/api/projects`, {
    headers: { Origin: base }, data: { key: "RDM", name: "Product delivery" }
  });
  assert.equal(projectResponse.status(), 201);
  const project = await projectResponse.json();
  await page.goto(`${base}/settings/integration`);
  await page.getByLabel("Connection name", { exact: true }).fill("Team Redmine");
  await page.getByLabel("Redmine base URL", { exact: true }).fill(fixtureUrl);
  await page.getByLabel("Personal API key", { exact: true }).fill("public-test-key");
  const saving = page.waitForResponse((response) => response.url().endsWith("/api/integration/redmine") && response.request().method() === "POST");
  await page.getByRole("region", { name: "Redmine connections" }).getByRole("button", { name: "Save connection", exact: true }).click();
  const saved = await saving;
  assert.equal(saved.status(), 200, `connection save returned ${saved.status()}`);
  await page.getByText("Team Redmine", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Check account", exact: true }).click();
  await page.getByText("Authenticated Redmine account: Morgan Lee", { exact: true }).waitFor();
  const screenshotDir = join(root, "docs/screenshots/kanso-3");
  await mkdir(screenshotDir, { recursive: true });
  await page.screenshot({ path: join(screenshotDir, "settings-desktop.png"),
    clip: { x: 0, y: 0, width: 1280, height: 695 } });
  await page.getByRole("button", { name: "Imports", exact: true }).click();
  await page.getByLabel("Destination project", { exact: true }).selectOption(project.id);
  await page.getByLabel("Provider", { exact: true }).selectOption("redmine");
  const source = page.getByLabel("Source", { exact: true });
  await source.selectOption({ label: "Team Redmine" });
  await page.getByText("Authenticated Redmine account: Morgan Lee", { exact: true }).waitFor();
  await page.getByLabel("Redmine project", { exact: true }).selectOption("4");
  await page.getByLabel("Assignee", { exact: true }).selectOption("me");
  await page.getByLabel("Watched", { exact: true }).selectOption("me");
  await page.getByLabel("Combine assignee and watched", { exact: true }).selectOption("or");
  await page.screenshot({ path: join(screenshotDir, "filters-desktop.png"), fullPage: true });
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  await page.getByText("Investigate API response latency", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Import selected", exact: true }).scrollIntoViewIfNeeded();
  // Crop below the fixed header and the partially scrolled preceding field.
  await page.screenshot({ path: join(screenshotDir, "preview-desktop.png"),
    clip: { x: 0, y: 84, width: 1280, height: 816 } });
  await page.getByLabel("Select Investigate API response latency", { exact: true }).check();
  await page.getByRole("button", { name: "Import selected", exact: true }).click();
  await page.getByText("RDM-1", { exact: true }).waitFor();
  await page.getByLabel("Issue state", { exact: true }).selectOption("closed");
  assert.equal(await page.getByRole("button", { name: "Import selected", exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  const runner = spawn("python3", [join(root, "scripts/test-redmine-live.py")], {
    cwd: root, env: { ...process.env, REDMINE_URL: fixtureUrl, REDMINE_API_KEY: "public-test-key" },
    stdio: "ignore"
  });
  const runnerStatus = await new Promise((resolve, reject) => {
    runner.once("error", reject);
    runner.once("exit", resolve);
  });
  assert.equal(runnerStatus, 0, "live runner failed against the synthetic fixture");
  console.log("Redmine browser workflow passed; screenshots saved in docs/screenshots/kanso-3/");
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
  await new Promise((resolve) => fixture.close(resolve));
}
