// Optional browser regression check using a disposable local Kanso instance.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("..", import.meta.url));
const cache = join(homedir(), ".cache/tmp/opencode");
await mkdir(cache, { recursive: true });
const temp = await mkdtemp(join(cache, "kanso-filter-browser-"));
const socket = createServer();
await new Promise((resolve) => socket.listen(0, "127.0.0.1", resolve));
const port = socket.address().port;
await new Promise((resolve) => socket.close(resolve));
const base = `http://127.0.0.1:${port}`;
const password = "disposable-filter-browser-test";
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
    const timeout = setTimeout(() => reject(new Error("Local Kanso did not start")), 15000);
    const read = (chunk) => {
      output += chunk;
      if (output.includes("kanso listening")) { clearTimeout(timeout); resolve(); }
    };
    server.stdout.on("data", read);
    server.stderr.on("data", read);
    server.once("error", (error) => { clearTimeout(timeout); reject(error); });
    server.once("exit", () => { clearTimeout(timeout); reject(new Error("Local Kanso exited")); });
  });
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(base);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "Imports", exact: true }).waitFor();
  const headers = { Origin: base };
  async function create(path, data) {
    const response = await page.request.post(`${base}${path}`, { headers, data });
    assert.equal(response.status(), 201, await response.text());
    return response.json();
  }
  await create("/api/projects", { key: "ONE", name: "First project" });
  await create("/api/projects", { key: "TWO", name: "Second project" });
  const label = await create("/api/labels", { name: "Urgent", color: "#dc2626" });
  await create("/api/labels", { name: "Unused", color: "#2563eb" });
  await create("/api/tasks", { project_key: "ONE", title: "Fix login", labels: [label.id] });
  await create("/api/tasks", { project_key: "ONE", title: "Write guide" });
  await page.goto(`${base}/p/ONE`);
  const filter = page.getByLabel("Filter by label");
  await filter.waitFor();
  await page.getByText("Fix login", { exact: true }).waitFor();
  assert.equal(await filter.locator("option").count(), 3);
  await filter.selectOption(label.id);
  assert.equal(await page.getByText("Write guide", { exact: true }).count(), 0);
  assert.equal(await page.locator(".board-scroll.filtered").count(), 1);
  assert.equal(await page.locator(".column").first().locator(".column-count").textContent(), "1");
  const card = page.locator(".card").first();
  const box = await card.boundingBox();
  await page.mouse.move(box.x + 20, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + 150, box.y + 100, { steps: 5 });
  assert.equal(await page.locator(".dragging").count(), 0);
  await page.mouse.up();
  const renamed = await page.request.patch(`${base}/api/labels/${label.id}`, {
    headers, data: { name: "Critical" }
  });
  assert.ok(renamed.ok());
  await page.getByText("Critical", { exact: true }).waitFor();
  await page.getByLabel("Search cards…").fill("guide");
  assert.equal(await page.locator(".board-scroll").getByText("Fix login", { exact: true }).count(), 0);
  await page.getByLabel("Search cards…").fill("");
  await page.reload();
  await filter.waitFor();
  await page.getByText("Fix login", { exact: true }).waitFor();
  assert.equal(await filter.inputValue(), label.id);
  if (process.env.KANSO_FILTER_SCREENSHOT) {
    await page.screenshot({ path: process.env.KANSO_FILTER_SCREENSHOT, animations: "disabled" });
  }
  await page.goto(`${base}/global`);
  await page.getByLabel("Show all columns").check();
  assert.equal(await filter.isVisible(), false);
  await page.goto(`${base}/p/ONE`);
  await page.getByText("Fix login", { exact: true }).waitFor();
  assert.equal(await filter.inputValue(), label.id);
  await page.goto(`${base}/p/TWO`);
  await filter.waitFor();
  assert.equal(await filter.inputValue(), "");
  await page.goto(`${base}/p/ONE`);
  await page.getByText("Fix login", { exact: true }).waitFor();
  assert.equal(await filter.inputValue(), "");
  await filter.selectOption(label.id);
  const deleted = await page.request.delete(`${base}/api/labels/${label.id}`, { headers });
  assert.ok(deleted.ok());
  await page.waitForFunction(() => document.querySelector(".label-filter")?.value === "");
  await page.getByText("Write guide", { exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log("Label filter browser check passed.");
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
  await new Promise((resolve) => { if (server.exitCode !== null) resolve(); else server.once("exit", resolve); });
}
