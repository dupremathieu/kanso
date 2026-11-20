// Disposable browser check and screenshot for parent project boards.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("..", import.meta.url));
const cache = join(homedir(), ".cache/tmp/opencode");
await mkdir(cache, { recursive: true });
const temp = await mkdtemp(join(cache, "kanso-parent-browser-"));
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
const exited = new Promise((resolve) => server.once("close", resolve));
let browser;
try {
  await new Promise((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(() => reject(new Error("Local Kanso did not start")), 15000);
    const read = (chunk) => {
      output += chunk;
      if (output.includes("kanso listening")) {
        clearTimeout(timeout);
        resolve();
      }
    };
    server.stdout.on("data", read);
    server.stderr.on("data", read);
    server.once("error", (error) => { clearTimeout(timeout); reject(error); });
    server.once("exit", () => { clearTimeout(timeout); reject(new Error("Local Kanso exited")); });
  });

  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1680, height: 880 }, colorScheme: "light" });
  page.setDefaultTimeout(10000);
  await page.goto(base);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "Imports", exact: true }).waitFor();
  const headers = { Origin: base };
  async function createProject(data) {
    const response = await page.request.post(`${base}/api/projects`, { headers, data });
    assert.equal(response.status(), 201, await response.text());
    return response.json();
  }
  const parent = await createProject({ key: "PROD", name: "Product launch", color: "#2563eb" });
  const web = await createProject({ key: "WEB", name: "Website", color: "#db2777" });
  await createProject({ key: "DOCS", name: "Documentation", color: "#059669", parent: "PROD" });
  await createProject({ key: "OTHER", name: "Unrelated project" });
  await page.goto(`${base}/settings/projects`);
  const webRow = page.locator(".settings-row").filter({ has: page.getByText("Website", { exact: true }) });
  await webRow.getByRole("button", { name: "Edit", exact: true }).click();
  const dialog = page.getByRole("dialog");
  const parentSelect = dialog.getByRole("combobox", { name: "Parent project", exact: true });
  await parentSelect.waitFor();
  assert.equal(await parentSelect.locator("option").filter({ hasText: "Documentation" }).count(), 0);
  await parentSelect.selectOption(parent.id);
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await webRow.getByText("Parent project: PROD — Product launch").waitFor();

  const cards = [
    ["PROD", "backlog", "Collect launch feedback"],
    ["PROD", "in_progress", "Coordinate release"],
    ["PROD", "done", "Approve launch scope"],
    ["WEB", "todo", "Add accessibility checks"],
    ["WEB", "in_review", "Review landing page"],
    ["WEB", "backlog", "Use legacy signup form"],
    ["DOCS", "backlog", "Translate release guide"],
    ["DOCS", "blocker", "Confirm migration details"],
    ["OTHER", "todo", "Unrelated card"]
  ];
  for (const [project_key, status, title] of cards) {
    const response = await page.request.post(`${base}/api/tasks`, {
      headers, data: { project_key, title, status }
    });
    assert.equal(response.status(), 201, await response.text());
  }
  const dropped = await page.request.post(`${base}/api/tasks/WEB-3/move`, {
    headers, data: { status: "dropped" }
  });
  assert.equal(dropped.status(), 200, await dropped.text());

  await page.goto(`${base}/p/PROD`);
  const scope = page.getByLabel("Board scope");
  await scope.waitFor();
  assert.equal(await scope.inputValue(), "project");
  assert.equal(await page.locator(".column").count(), 7);
  assert.equal(await page.getByText("Review landing page").count(), 0);
  await scope.selectOption("parent");
  const groups = page.locator(".project-group");
  await groups.nth(2).waitFor();
  assert.deepEqual(await groups.locator("h2").allTextContents(), ["Product launch", "Website", "Documentation"]);
  for (const group of await groups.all()) {
    assert.equal(await group.locator(".column").count(), 7);
    assert.equal(await group.locator('.column[data-status="dropped"]').count(), 1);
  }
  const screenshot = process.env.KANSO_PARENT_SCREENSHOT;
  if (screenshot) {
    await mkdir(dirname(screenshot), { recursive: true });
    await page.screenshot({ path: screenshot, animations: "disabled" });
  }

  const search = page.locator(".search-input");
  await search.fill("legacy");
  assert.equal(await page.locator(".card").count(), 1);
  await page.getByText("Use legacy signup form").waitFor();
  await search.clear();

  await page.getByText("Review landing page").click();
  await page.waitForURL("**/p/WEB/task/2?parent=PROD");
  await groups.nth(2).waitFor();
  await page.getByRole("button", { name: "Close panel" }).click();
  await page.waitForURL("**/p/PROD?children=1");
  await groups.nth(2).waitFor();

  await page.goto(`${base}/p/WEB/task/999?parent=PROD`);
  await page.getByRole("link", { name: "Back to the board" }).click();
  await page.waitForURL("**/p/PROD?children=1");
  await groups.nth(2).waitFor();

  const webBoard = groups.filter({ has: page.getByRole("heading", { name: "Website" }) });
  const prodBoard = groups.filter({ has: page.getByRole("heading", { name: "Product launch" }) });
  const newColumn = webBoard.locator('.column[data-status="backlog"]');
  await newColumn.getByRole("button", { name: "Add a card" }).click();
  await newColumn.locator("input").fill("Check link targets");
  await newColumn.locator("input").press("Enter");
  const newCard = newColumn.getByText("Check link targets");
  await newCard.waitFor();
  assert.equal((await (await page.request.get(`${base}/api/tasks/WEB-4`)).json()).title, "Check link targets");

  async function dragCardThrough(...columns) {
    const card = await webBoard.getByText("Check link targets").boundingBox();
    await page.mouse.move(card.x + 8, card.y + card.height / 2);
    await page.mouse.down();
    for (const column of columns) {
      const target = await column.locator(".column-cards").boundingBox();
      await page.mouse.move(target.x + 20, target.y + 12, { steps: 15 });
    }
    await page.mouse.up();
  }
  const webTodo = webBoard.locator('.column[data-status="todo"]');
  await dragCardThrough(webTodo, prodBoard.locator('.column[data-status="todo"]'));
  await newCard.waitFor();
  const moved = page.waitForResponse((response) => response.url().endsWith("/api/tasks/WEB-4/move") && response.ok());
  await dragCardThrough(webTodo);
  await webTodo.getByText("Check link targets").waitFor();
  await moved;

  // Project events update the expanded board without a manual reload.
  const detached = await page.request.patch(`${base}/api/projects/${web.id}`, {
    headers, data: { parent: null }
  });
  assert.equal(detached.status(), 200, await detached.text());
  await webBoard.waitFor({ state: "detached" });
  const archived = await page.request.post(`${base}/api/projects/DOCS/archive`, { headers });
  assert.equal(archived.status(), 200, await archived.text());
  await groups.first().waitFor({ state: "detached" });
  await scope.waitFor({ state: "hidden" });
  assert.equal(await page.locator(".column").count(), 7);
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByText("Unrelated card").waitFor({ state: "hidden" });
  await page.getByRole("checkbox", { name: "Show all columns" }).check();
  await page.getByText("Unrelated card").waitFor();
  assert.equal(await page.locator('.column[data-status="dropped"]').count(), 0);
  console.log("Parent project browser check passed.");
} finally {
  if (browser) await browser.close();
  server.kill("SIGTERM");
  await exited;
  await rm(temp, { recursive: true, force: true });
}
