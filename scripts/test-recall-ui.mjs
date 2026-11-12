// Optional browser regression against disposable local data and a mock adapter.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir } from "node:fs/promises";
import { createServer } from "node:http";
import { createServer as createSocket } from "node:net";
import { join } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("..", import.meta.url));
const tmp = join(homedir(), ".cache/tmp/opencode");
await mkdir(tmp, { recursive: true });
const dir = await mkdtemp(join(tmp, "kanso-recall-ui-"));
const sends = [];
let failed = true;
const mock = createServer(async (request, response) => {
  let raw = "";
  for await (const chunk of request) raw += chunk;
  const data = raw ? JSON.parse(raw) : {};
  response.setHeader("Content-Type", "application/json");
  if (data.action === "session.send") {
    sends.push(data);
    response.end(JSON.stringify({ promptDispatched: !failed }));
  } else if (data.action === "session.list") {
    response.end(JSON.stringify({ sessions: [{ id: "ses_ui", status: "idle", directory: "/local" }] }));
  } else response.end(JSON.stringify({ version: "test", openchamberVersion: "2.6.0" }));
});
await new Promise(resolve => mock.listen(0, "127.0.0.1", resolve));
const socket = createSocket();
await new Promise(resolve => socket.listen(0, "127.0.0.1", resolve));
const port = socket.address().port;
await new Promise(resolve => socket.close(resolve));
const base = `http://127.0.0.1:${port}`;
const env = { ...process.env, HOME: dir, KANSO_DATA_DIR: join(dir, "data"), KANSO_BIND: `127.0.0.1:${port}`,
  KANSO_PASSWORD: "disposable-test", KANSO_OPENCHAMBER_URL: `http://127.0.0.1:${mock.address().port}`,
  KANSO_OPENCHAMBER_TOKEN: "local-test" };
const server = spawn(join(root, "target/debug/kanso"), ["serve"], { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] });
let browser;
try {
  await new Promise((resolve, reject) => {
    let output = "";
    const timer = setTimeout(() => reject(new Error("server startup timeout")), 15000);
    const read = data => { output += data; if (output.includes("kanso listening")) { clearTimeout(timer); resolve(); } };
    server.stdout.on("data", read); server.stderr.on("data", read);
    server.once("exit", () => { clearTimeout(timer); reject(new Error("server exited")); });
  });
  const login = await fetch(`${base}/api/auth/login`, { method: "POST", headers: { "Content-Type": "application/json", Origin: base }, body: JSON.stringify({ password: env.KANSO_PASSWORD }) });
  assert.equal(login.status, 204);
  const cookie = login.headers.get("set-cookie").split(";")[0];
  async function api(path, body) {
    const res = await fetch(`${base}${path}`, { method: "POST", headers: { "Content-Type": "application/json", Origin: base, Cookie: cookie }, body: JSON.stringify(body) });
    assert.ok(res.ok, `${path}: ${res.status}`);
    return res.json();
  }
  const project = await api("/api/projects", { key: "UIT", name: "Recall browser test" });
  const card = await api("/api/tasks", { project_id: project.id, title: "Recall test" });
  await api(`/api/tasks/${card.reference}/session`, { session_id: "ses_ui", directory: "/local", status: "idle" });
  await api(`/api/tasks/${card.reference}/move`, { status: "in_review" });
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  assert.equal((await page.request.post(`${base}/api/auth/login`, { headers: { Origin: base }, data: { password: env.KANSO_PASSWORD } })).status(), 204);
  await page.goto(`${base}/p/UIT/task/1`);
  await page.getByRole("button", { name: "Re-call agent", exact: true }).click();
  await page.getByRole("textbox", { name: "Instructions for the agent…" }).fill("Please fix review findings");
  await page.getByRole("button", { name: "Send instructions" }).click();
  await page.getByText("Please fix review findings", { exact: true }).waitFor();
  await page.locator(".comment-retry").waitFor();
  assert.equal(sends.length, 1);
  failed = false;
  await page.locator(".comment-retry").click();
  await page.waitForFunction(() => !document.querySelector(".comment-retry"));
  assert.equal(sends.length, 2);
  assert.deepEqual(sends[0], sends[1]);
  assert.deepEqual(errors, []);
  console.log("PASS: recall form, failed delivery, explicit retry and successful delivery");
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { const exited = new Promise(resolve => server.once("exit", resolve)); server.kill("SIGTERM"); await exited; }
  await new Promise(resolve => mock.close(resolve));
}
