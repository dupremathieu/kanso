// Optional real-browser check; only disposable local data is modified.
// Build first. Set PLAYWRIGHT_MODULE to externally installed Playwright and
// KANSO_LABEL_SCREENSHOTS to retain desktop/mobile captures.
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
const temp = await mkdtemp(join(cache, "kanso-label-browser-"));
const socket = createServer();
await new Promise((resolve) => socket.listen(0, "127.0.0.1", resolve));
const port = socket.address().port;
await new Promise((resolve) => socket.close(resolve));
const base = `http://127.0.0.1:${port}`;
const password = "disposable-label-browser-test";
const server = spawn(join(root, "target/debug/kanso"), ["serve"], {
  cwd: root,
  env: { ...process.env, KANSO_BIND: `127.0.0.1:${port}`, KANSO_PUBLIC_URL: base,
    KANSO_DATA_DIR: join(temp, "data"), KANSO_PASSWORD: password,
    KANSO_OPENCHAMBER_URL: "", KANSO_OPENCHAMBER_TOKEN: "", KANSO_LOG: "info" },
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
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(base);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "Imports", exact: true }).waitFor();
  const headers = { Origin: base };
  const projectResponse = await page.request.post(`${base}/api/projects`, {
    headers, data: { key: "UIT", name: "Label templates" }
  });
  assert.equal(projectResponse.status(), 201);
  const labelResponse = await page.request.post(`${base}/api/labels`, {
    headers, data: { name: "security", color: "#2563eb", prompt_template: "Review {{ref}} for security risks.\nReport findings and add regression tests." }
  });
  assert.equal(labelResponse.status(), 201);
  const label = await labelResponse.json();
  await page.goto(`${base}/settings/projects`);
  const labelRow = page.locator("article.settings-row").filter({ hasText: "security" });
  await labelRow.getByText("Prompt template set", { exact: true }).waitFor();
  const editLabel = page.locator("article.settings-row").filter({ hasText: "security" }).getByRole("button", { name: "Edit", exact: true });
  await editLabel.click();
  const template = page.getByLabel(/^Prompt template/);
  assert.equal(await template.inputValue(), label.prompt_template);
  if (process.env.KANSO_LABEL_SCREENSHOTS) {
    await mkdir(process.env.KANSO_LABEL_SCREENSHOTS, { recursive: true });
    await page.screenshot({ path: join(process.env.KANSO_LABEL_SCREENSHOTS, "label-template-desktop.png"), animations: "disabled" });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: join(process.env.KANSO_LABEL_SCREENSHOTS, "label-template-mobile.png"), animations: "disabled" });
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
    assert.equal(overflow, false, "mobile must not overflow horizontally");
  }
  await template.fill("Updated {{project}}");
  await page.getByRole("dialog").getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  let labels = await (await page.request.get(`${base}/api/labels`)).json();
  assert.equal(labels.find((item) => item.id === label.id).prompt_template, "Updated {{project}}");
  await labelRow.getByText("Prompt template set", { exact: true }).waitFor();
  await editLabel.click();
  await template.fill("");
  await page.getByRole("dialog").getByRole("button", { name: "Edit", exact: true }).click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  labels = await (await page.request.get(`${base}/api/labels`)).json();
  assert.equal(labels.find((item) => item.id === label.id).prompt_template, null);
  await labelRow.getByText("Prompt template set", { exact: true }).waitFor({ state: "hidden" });
  assert.deepEqual(errors, []);
  console.log("Label template browser check passed (edit, persistence, clear, screenshots).");
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
  await new Promise((resolve) => { if (server.exitCode !== null) resolve(); else server.once("exit", resolve); });
}
