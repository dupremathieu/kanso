// Optional real-browser check for the GitHub import title prefix. Build
// target/debug/kanso first and set PLAYWRIGHT_MODULE if Playwright is not
// installed locally. It uses a private fake gh and synthetic data only; it
// never runs the installed gh or reads real credential files.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, writeFile, chmod, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("..", import.meta.url));
const baseTemp = join(homedir(), ".cache/tmp/opencode");
await mkdir(baseTemp, { recursive: true });
const temp = await mkdtemp(join(baseTemp, "kanso-github-import-prefix-"));
const bin = join(temp, "bin");
await mkdir(bin);
const user = { login: "me", type: "User" };
const issue = {
  node_id: "fixture-issue-547",
  number: 547,
  title: "The application fails to compile on Ubuntu 26.04",
  body: "The build stops with a linker error.",
  state: "open",
  user: { login: "alice", type: "User" },
  assignees: [user]
};
const review = {
  node_id: "fixture-pr-1547",
  number: 1547,
  title: "Add the new remove parameter",
  body: "Adds the parameter and updates its documentation.",
  state: "open",
  user: { login: "alice", type: "User" },
  assignees: [],
  pull_request: { url: "https://api.github.com/repos/fixture/repository/pulls/1547" },
  draft: false,
  merged_at: null,
  requested_reviewers: [user]
};
const follow = {
  node_id: "fixture-pr-4756",
  number: 4756,
  title: "Documentation refortoring",
  body: "Reorganises the operator documentation.",
  state: "open",
  user,
  assignees: [],
  pull_request: { url: "https://api.github.com/repos/fixture/repository/pulls/4756" },
  draft: true,
  merged_at: null,
  requested_reviewers: []
};
const fake = join(bin, "gh");
await writeFile(join(bin, "user.json"), JSON.stringify(user));
await writeFile(join(bin, "issues.json"), JSON.stringify([issue]));
await writeFile(join(bin, "pulls.json"), JSON.stringify([review, follow]));
await writeFile(fake, `#!/bin/sh
endpoint=""
for arg in "$@"; do endpoint="$arg"; done
case "$endpoint" in
  user) cat "$(dirname "$0")/user.json" ;;
  repos/fixture/repository/issues*) cat "$(dirname "$0")/issues.json" ;;
  repos/fixture/repository/pulls*) cat "$(dirname "$0")/pulls.json" ;;
  *) exit 1 ;;
esac
`, { mode: 0o700 });
await chmod(fake, 0o700);
const socket = createServer();
await new Promise(resolve => socket.listen(0, "127.0.0.1", resolve));
const port = socket.address().port;
await new Promise(resolve => socket.close(resolve));
const origin = `http://127.0.0.1:${port}`;
const password = "fixture-password";
const env = {
  ...process.env,
  HOME: temp,
  PATH: `${bin}:${process.env.PATH}`,
  GH_CONFIG_DIR: join(temp, "gh-config"),
  KANSO_DATA_DIR: join(temp, "data"),
  KANSO_BIND: `127.0.0.1:${port}`,
  KANSO_PASSWORD: password
};
for (const key of [
  "GH_TOKEN",
  "GITHUB_TOKEN",
  "GH_ENTERPRISE_TOKEN",
  "GITHUB_ENTERPRISE_TOKEN",
  "GH_HOST",
  "KANSO_OPENCHAMBER_URL",
  "KANSO_OPENCHAMBER_TOKEN",
  "KANSO_PUBLIC_URL"
]) delete env[key];
const server = spawn(join(root, "target/debug/kanso"), ["serve"], {
  cwd: root, env, stdio: ["ignore", "pipe", "pipe"]
});
let serverError = null;
server.on("error", error => { serverError = error; });
let browser;
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    if (serverError || server.exitCode !== null || server.signalCode !== null) break;
    try { if ((await fetch(`${origin}/api/health`)).ok) { ready = true; break; } } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(ready, "isolated fixture server did not start");
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(origin);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "Imports", exact: true }).waitFor();

  const created = await page.request.post(`${origin}/api/projects`, {
    headers: { Origin: origin },
    data: { key: "GHI", name: "GitHub import prefix verification" }
  });
  assert.equal(created.status(), 201);
  const project = await created.json();
  const configured = await page.request.put(`${origin}/api/projects/${project.id}/github-repositories`, {
    headers: { Origin: origin },
    data: [{ repository: "fixture/repository", issues: true, review: true, follow: true }]
  });
  assert.equal(configured.status(), 200);

  await page.goto(`${origin}/p/GHI`);
  await page.getByRole("button", { name: "Imports", exact: true }).click();
  const destination = page.getByLabel("Destination project", { exact: true });
  const provider = page.getByLabel("Provider", { exact: true });
  const source = page.getByLabel("Source", { exact: true });
  const category = page.getByLabel("GitHub category", { exact: true });
  const preview = page.getByRole("button", { name: "Preview", exact: true });
  await destination.selectOption(project.id);
  await provider.locator('option[value="github"]').waitFor({ state: "attached" });
  await provider.selectOption("github");
  await source.locator('option[value="fixture/repository"]').waitFor({ state: "attached" });
  await source.selectOption("fixture/repository");
  await category.locator('option[value="issues"]').waitFor({ state: "attached" });

  const expected = {
    issues: `Issue #547 - ${issue.title}`,
    review: `Review PR #1547 - ${review.title}`,
    follow: `Follow PR #4756 - ${follow.title}`
  };
  const screenshots = process.env.KANSO_GITHUB_IMPORT_SCREENSHOTS;
  if (screenshots) await mkdir(resolve(screenshots), { recursive: true });

  for (const [name, title] of Object.entries(expected)) {
    await category.selectOption(name);
    assert.equal(await preview.isEnabled(), true);
    await preview.click();
    await page.getByRole("heading", { name: "Preview", exact: true }).waitFor();
    const choice = page.getByRole("checkbox", { name: `Select ${title}`, exact: true });
    await choice.waitFor();
    if (name === "issues") {
      await page.evaluate(() => new Promise(requestAnimationFrame));
      if (screenshots) {
        await page.screenshot({ path: join(resolve(screenshots), "preview-desktop.png"), fullPage: true });
      }
    }
    await choice.check();
    await page.getByRole("button", { name: "Import selected", exact: true }).click();
    await page.getByRole("status").locator("p").filter({ hasText: `${title}: created` }).waitFor();
  }

  await page.goto(`${origin}/p/GHI`);
  for (const title of Object.values(expected)) {
    await page.getByText(title, { exact: true }).waitFor();
  }
  await page.evaluate(() => new Promise(requestAnimationFrame));
  if (screenshots) {
    await page.screenshot({ path: join(resolve(screenshots), "board-desktop.png"), fullPage: true });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  if (screenshots) {
    await page.screenshot({ path: join(resolve(screenshots), "board-mobile.png"), fullPage: true });
  }
  assert.deepEqual(errors, []);
  console.log("GitHub import prefix browser verification passed (issues, review, follow, desktop/mobile).");
} finally {
  if (browser) await browser.close();
  if (!serverError && server.exitCode === null && server.signalCode === null) {
    const exited = new Promise(resolve => server.once("exit", resolve));
    server.kill("SIGTERM");
    await exited;
  }
  await rm(temp, { recursive: true, force: true });
}
