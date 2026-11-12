// Disposable browser check and screenshots for project moves.
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
const temp = await mkdtemp(join(cache, "kanso-move-browser-"));
const socket = createServer();
await new Promise((resolve) => socket.listen(0, "127.0.0.1", resolve));
const port = socket.address().port;
await new Promise((resolve) => socket.close(resolve));
const base = `http://127.0.0.1:${port}`;
const password = "disposable-browser-test";
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
    server.once("error", reject);
    server.once("exit", () => reject(new Error("Local Kanso exited")));
  });
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(base);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "Imports", exact: true }).waitFor();
  const headers = { Origin: base };
  for (const project of [{ key: "DES", name: "Design" }, { key: "WEB", name: "Website" }]) {
    assert.equal((await page.request.post(`${base}/api/projects`, { headers, data: project })).status(), 201);
  }
  assert.equal((await page.request.post(`${base}/api/tasks`, { headers, data: {
    project_key: "DES", title: "Polish the landing page", description: "Review the layout and prepare it for launch."
  } })).status(), 201);
  await page.goto(`${base}/p/DES/task/1`);
  const selector = page.getByLabel("Move to project…");
  await selector.waitFor();
  const screenshots = process.env.KANSO_MOVE_SCREENSHOTS;
  if (screenshots) {
    await mkdir(screenshots, { recursive: true });
    await selector.scrollIntoViewIfNeeded().catch(() => selector.scrollIntoViewIfNeeded());
    await page.screenshot({ path: join(screenshots, "project-move-desktop.png"), animations: "disabled" });
    await page.setViewportSize({ width: 390, height: 844 });
    await selector.scrollIntoViewIfNeeded().catch(() => selector.scrollIntoViewIfNeeded());
    await page.screenshot({ path: join(screenshots, "project-move-mobile.png"), animations: "disabled" });
  }
  const option = await selector.locator("option").filter({ hasText: "Website" }).getAttribute("value");
  await selector.selectOption(option);
  await page.getByText("WEB-1", { exact: true }).first().waitFor();
  assert.equal((await (await page.request.get(`${base}/api/tasks/DES-1`)).json()).reference, "WEB-1");
  console.log("Project move browser check passed.");
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
}
