// Disposable browser check and screenshots for removing a comment.
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
const temp = await mkdtemp(join(cache, "kanso-comment-browser-"));
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
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();
  await page.goto(base);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "Imports", exact: true }).waitFor();
  const headers = { Origin: base };
  assert.equal((await page.request.post(`${base}/api/projects`, { headers, data: { key: "CMT", name: "Comments" } })).status(), 201);
  assert.equal((await page.request.post(`${base}/api/tasks`, { headers, data: {
    project_key: "CMT", title: "Comment thread"
  } })).status(), 201);
  for (const body of ["First to remove.", "Second stays."]) {
    const created = await page.request.post(`${base}/api/tasks/CMT-1/comment`, { headers, data: { body } });
    assert.equal(created.status(), 201);
  }

  await page.goto(`${base}/p/CMT/task/1`);
  await page.getByRole("heading", { name: "Comment thread", exact: true }).waitFor();
  await page.getByText("First to remove.", { exact: true }).waitFor();
  const removeFor = (text) => page
    .locator("article.comment", { hasText: text })
    .getByRole("button", { name: "Remove comment", exact: true });

  // A second tab with the same card open follows the deletion through SSE.
  const other = await context.newPage();
  const stream = other.waitForResponse(
    (response) => response.url().includes("/api/events") && response.status() === 200
  );
  await other.goto(`${base}/p/CMT/task/1`);
  await stream;
  await other.getByText("First to remove.", { exact: true }).waitFor();

  await removeFor("First to remove.").click();
  await page.getByText("First to remove.", { exact: true }).waitFor({ state: "detached" });
  await page.getByText("removed a comment", { exact: true }).waitFor();
  await other.getByText("First to remove.", { exact: true }).waitFor({ state: "detached" });
  await other.getByText("Second stays.", { exact: true }).waitFor();

  // The remaining comment keeps a working button after the refresh.
  const remaining = removeFor("Second stays.");
  assert.equal(await remaining.isEnabled(), true);
  await remaining.click();
  await page.getByText("No comments yet.", { exact: true }).waitFor();
  const detail = await (await page.request.get(`${base}/api/tasks/CMT-1`)).json();
  assert.equal(detail.comments.length, 0);

  // A removed comment leaves the composer usable for the next one.
  await page.getByLabel("Write a comment…", { exact: true }).fill("Third comment");
  await page.getByRole("button", { name: "Comment", exact: true }).click();
  await page.getByText("Third comment", { exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Remove comment", exact: true }).count(), 1);
  await page.reload();
  await page.getByText("Third comment", { exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Remove comment", exact: true }).count(), 1);

  const screenshots = process.env.KANSO_COMMENT_SCREENSHOTS;
  if (screenshots) {
    await mkdir(screenshots, { recursive: true });
    await page.screenshot({ path: join(screenshots, "comment-remove-desktop.png"), animations: "disabled" });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`${base}/p/CMT/task/1`);
    await page.getByText("Third comment", { exact: true }).waitFor();
    await page.screenshot({ path: join(screenshots, "comment-remove-mobile.png"), fullPage: true, animations: "disabled" });
  }
  console.log("Comment removal browser check passed.");
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
}
