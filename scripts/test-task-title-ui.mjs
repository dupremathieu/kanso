// Disposable browser check and screenshots for editing a task title.
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
const temp = await mkdtemp(join(cache, "kanso-title-browser-"));
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
  assert.equal((await page.request.post(`${base}/api/projects`, { headers, data: { key: "TIT", name: "Titles" } })).status(), 201);
  assert.equal((await page.request.post(`${base}/api/tasks`, { headers, data: {
    project_key: "TIT", title: "Original title", description: "Keep this description."
  } })).status(), 201);
  assert.equal((await page.request.post(`${base}/api/tasks`, { headers, data: {
    project_key: "TIT", title: "Second title"
  } })).status(), 201);
  await page.goto(`${base}/p/TIT/task/1`);
  await page.getByRole("heading", { name: "Original title", exact: true }).waitFor();

  const edit = page.getByRole("button", { name: "Edit title", exact: true });
  await edit.click();
  const input = page.getByLabel("Card title", { exact: true });
  await input.waitFor();
  assert.equal(await input.inputValue(), "Original title");
  assert.equal(
    await page.evaluate(() => document.activeElement?.getAttribute("data-title-editor")),
    "true",
    "the title input should receive focus when the editor opens"
  );

  // Escape leaves the title untouched, including from the Save/Cancel buttons.
  await input.fill("Discarded title");
  await input.press("Escape");
  await page.getByRole("heading", { name: "Original title", exact: true }).waitFor();
  assert.equal((await (await page.request.get(`${base}/api/tasks/TIT-1`)).json()).title, "Original title");
  assert.equal(
    await page.evaluate(() => document.activeElement?.getAttribute("data-focus-key")),
    "title-edit",
    "focus should return to the Edit title button"
  );
  await edit.click();
  await page.getByRole("button", { name: "Cancel", exact: true }).focus();
  await page.keyboard.press("Escape");
  await page.getByRole("heading", { name: "Original title", exact: true }).waitFor();
  assert.equal((await (await page.request.get(`${base}/api/tasks/TIT-1`)).json()).title, "Original title");

  // An empty title is rejected without a request.
  await edit.click();
  await input.fill("   ");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.getByText("A title is required.", { exact: true }).waitFor();
  assert.equal((await (await page.request.get(`${base}/api/tasks/TIT-1`)).json()).title, "Original title");

  // Save with the keyboard, then confirm persistence.
  await input.fill("A better title");
  await input.press("Enter");
  await page.getByRole("heading", { name: "A better title", exact: true }).waitFor();
  assert.equal((await (await page.request.get(`${base}/api/tasks/TIT-1`)).json()).title, "A better title");
  assert.equal((await (await page.request.get(`${base}/api/tasks/TIT-1`)).json()).description, "Keep this description.");
  await page.reload();
  await page.getByRole("heading", { name: "A better title", exact: true }).waitFor();

  // Opening another card while editing resets the editor for the new card.
  await edit.click();
  await page.getByRole("group", { name: "Open card TIT-2: Second title", exact: true }).click();
  await page.getByRole("heading", { name: "Second title", exact: true }).waitFor();
  assert.equal(await page.getByLabel("Card title", { exact: true }).count(), 0);
  await page.goto(`${base}/p/TIT/task/1`);
  await page.getByRole("heading", { name: "A better title", exact: true }).waitFor();

  const screenshots = process.env.KANSO_TITLE_SCREENSHOTS;
  if (screenshots) {
    await mkdir(screenshots, { recursive: true });
    await edit.click();
    await page.screenshot({ path: join(screenshots, "task-title-edit-desktop.png"), animations: "disabled" });
    await page.keyboard.press("Escape");
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`${base}/p/TIT/task/1`);
    await page.getByRole("heading", { name: "A better title", exact: true }).waitFor();
    await edit.click();
    await page.screenshot({ path: join(screenshots, "task-title-edit-mobile.png"), animations: "disabled" });
  }
  console.log("Task title browser check passed.");
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
}
