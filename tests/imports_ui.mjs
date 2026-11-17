// Lightweight DOM exercise for the import workflow; no cloud service needed.
import assert from "node:assert/strict";

class FakeNode {
  constructor(tag = "#text") { this.tag = tag; this.children = []; this.listeners = {}; this.value = ""; this.textContent = ""; this.disabled = false; this.checked = false; this.dataset = {}; this.style = {}; }
  append(...children) { for (const child of children) child.parent = this; this.children.push(...children); if (this.tag === "select" && !this.value && this.children.length) this.value = this.children[0].value; }
  replaceChildren(...children) { this.children = []; if (this.tag === "select") this.value = ""; this.append(...children); }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter((child) => child !== this); }
  replaceWith(node) { if (this.parent) { this.parent.children = this.parent.children.map((child) => child === this ? node : child); node.parent = this.parent; } }
  setAttribute(key, value) { this[key] = value; }
  addEventListener(name, fn) { (this.listeners[name] ||= []).push(fn); }
  querySelectorAll(tag) { return this.children.flatMap((child) => [ ...(child.tag === tag ? [child] : []), ...child.querySelectorAll(tag) ]); }
  async fire(name) { for (const fn of this.listeners[name] || []) await fn({ preventDefault() {} }); }
}
globalThis.Node = FakeNode;
globalThis.document = { createElement: (tag) => new FakeNode(tag), createTextNode: (text) => { const node = new FakeNode(); node.textContent = text; return node; } };
const { renderImports, sourceLink } = await import("../web/imports.js");
const walk = (node) => [node, ...node.children.flatMap(walk)];
const find = (root, predicate) => walk(root).find(predicate);
const calls = [];
let providers = [];
let labels = [];
let providersByProject = null;
let pendingPreview = null;
let pendingConfirm = null;
let pendingProviders = null;
let failure = false;
let labelFailure = false;
globalThis.fetch = async (path, options) => {
  calls.push({ path, payload: options.body && JSON.parse(options.body) });
  if (path.endsWith("preview") && pendingPreview) await pendingPreview;
  if (path.endsWith("confirm") && pendingConfirm) await pendingConfirm;
  if (path.includes("/imports/providers?") && pendingProviders) await pendingProviders;
  if (failure) return { ok: false, status: 503, headers: { get: () => "application/json" }, json: async () => ({ error: { code: "provider_error", message: "Provider unavailable" } }) };
  if (labelFailure && path === "/api/labels") return { ok: false, status: 503, headers: { get: () => "application/json" }, json: async () => ({ error: { code: "label_error", message: "Labels unavailable" } }) };
  const projectId = new URL(path, "https://test.invalid").searchParams.get("project_id");
  const body = path.includes("/discover") ? { account: { name: "Fixture User" }, projects: projectId === "project2" ? [{ id: 4, name: "Atlas" }, { id: 5, name: "Beacon" }] : [{ id: 5, name: "Beacon" }, { id: 4, name: "Atlas" }] } : path === "/api/labels" ? labels : path.includes("/imports/providers") ? (providersByProject?.[projectId] || providers) : path.endsWith("preview")
    ? { preview_id: "snapshot", candidates: [{ index: 0, valid: true, existing: null, candidate: { title: "<img onerror=alert(1)>", kind: "issue", source_url: "https://example.org/a" } }] }
    : [{ index: 0, status: "created", reference: "TST-1", message: null }];
  return { ok: true, status: 200, headers: { get: () => "application/json" }, json: async () => body };
};
const region = new FakeNode("main");
await renderImports(region, [], () => true, () => {});
assert.ok(find(region, (n) => n.textContent.includes("not yet available")));
assert.equal(sourceLink("javascript:alert(1)"), null);
assert.equal(sourceLink("https://user:password@example.org/"), null);
assert.equal(sourceLink("data:text/html,<script>alert(1)</script>"), null);
assert.equal(sourceLink("https://example.org/a").rel, "noopener noreferrer");
providers = [{ id: "fixture", name: "Fixture", sources: [{ id: "source", name: "Source" }], filters: [{ id: "query", label: "Query" }] }];
labels = [{ id: "global", name: "Global", is_global: true }, { id: "local", name: "Local", is_global: false, project_ids: ["project"] }, { id: "other", name: "Other", is_global: false, project_ids: ["other"] }, { id: "unknown", name: "Unknown scope" }];
await renderImports(region, [{ id: "project", key: "TST", name: "Test" }], () => true, () => {});
const project = find(region, (n) => n["aria-label"] === "Destination project");
const provider = find(region, (n) => n["aria-label"] === "Provider");
const source = find(region, (n) => n["aria-label"] === "Source");
const preview = find(region, (n) => n.textContent === "Preview" && n.tag === "button");
assert.equal(preview.disabled, true);
project.value = "project"; await project.fire("change");
const label = find(region, (n) => n["aria-label"] === "Label for new tasks");
assert.deepEqual(label.children.map((option) => option.value), ["", "global", "local"]);
provider.value = "fixture"; await provider.fire("change");
source.value = "source"; await source.fire("change");
label.value = "local";
label.selectedOptions = [label.children[2]];
await preview.fire("click");
assert.equal(calls.findLast((call) => call.path.endsWith("preview")).payload.label_id, "local");
assert.ok(find(region, (n) => n.textContent === "Label for new tasks: Local"));
label.value = ""; await label.fire("change");
assert.equal(find(region, (n) => n.textContent === "Import selected"), undefined);
label.value = "local";
await preview.fire("click");
const checkbox = find(region, (n) => n.tag === "input" && n.type === "checkbox");
const confirm = find(region, (n) => n.textContent === "Import selected");
assert.equal(checkbox.checked, false);
assert.equal(confirm.disabled, true);
assert.ok(find(region, (n) => n.textContent === "<img onerror=alert(1)>"));
checkbox.checked = true; await checkbox.fire("change");
assert.equal(confirm.disabled, false);
let releaseConfirm;
pendingConfirm = new Promise((resolve) => { releaseConfirm = resolve; });
const confirming = confirm.fire("click");
assert.equal(confirm.disabled, true);
assert.equal(checkbox.disabled, true);
await confirm.fire("click");
assert.equal(calls.filter((call) => call.path.endsWith("confirm")).length, 1);
releaseConfirm(); await confirming; pendingConfirm = null;
assert.deepEqual(calls.at(-1).payload.selected, [0]);
assert.ok(find(region, (n) => n.textContent === "TST-1"));
failure = true;
await confirm.fire("click");
assert.ok(find(region, (n) => n.textContent === "Provider unavailable"));
failure = false;
await confirm.fire("click");
assert.equal(find(region, (n) => n.role === "alert").textContent, "");
const filter = find(region, (n) => n["aria-label"] === "Query");
filter.value = "changed"; await filter.fire("input");
assert.equal(find(region, (n) => n.textContent === "Import selected"), undefined);
label.value = ""; await label.fire("change");
let releasePreview;
pendingPreview = new Promise((resolve) => { releasePreview = resolve; });
const previewing = preview.fire("click");
source.value = ""; await source.fire("change");
releasePreview(); await previewing; pendingPreview = null;
assert.equal(find(region, (n) => n.textContent === "Import selected"), undefined);
assert.equal(preview.disabled, true);
source.value = "source"; await source.fire("change");
failure = true;
await preview.fire("click");
assert.ok(find(region, (n) => n.textContent === "Provider unavailable"));
failure = false;
label.value = "global";
project.value = ""; await project.fire("change");
assert.equal(label.value, "");
assert.deepEqual(label.children.map((option) => option.value), [""]);
labelFailure = true;
project.value = "project"; await project.fire("change");
assert.equal(find(region, (n) => n.role === "alert").textContent, "Labels unavailable");
assert.deepEqual(label.children.map((option) => option.value), [""]);
provider.value = "fixture"; await provider.fire("change");
source.value = "source"; await source.fire("change");
assert.equal(preview.disabled, false);
await preview.fire("click");
assert.equal(calls.findLast((call) => call.path.endsWith("preview")).payload.label_id, null);
labelFailure = false;
failure = true;
await renderImports(region, [], () => true, () => {});
assert.ok(find(region, (n) => n.textContent === "Provider unavailable"));
failure = false;
await renderImports(region, [], () => true, () => {});
assert.ok(find(region, (n) => n.textContent.includes("Create an active project")));
providersByProject = null;
providers = [{ id: "redmine", name: "Redmine", sources: [{ id: "connection", name: "Fixture" }], filters: ["project", "assignee", "state", "watched", "operator"].map((id) => ({ id, label: id })) }];
await renderImports(region, [{ id: "project", key: "TST", name: "Test" }], () => true, () => {});
const rmProject = find(region, (n) => n["aria-label"] === "Destination project");
const rmProvider = find(region, (n) => n["aria-label"] === "Provider");
const rmSource = find(region, (n) => n["aria-label"] === "Source");
rmProject.value = "project"; await rmProject.fire("change");
rmProvider.value = "redmine"; await rmProvider.fire("change");
rmSource.value = "connection"; await rmSource.fire("change");
await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
assert.ok(find(region, (n) => n.textContent === "Atlas"));
assert.equal(calls.at(-1).path, "/api/integration/redmine/connection/discover?project_id=project");
assert.deepEqual(find(region, (n) => n["aria-label"] === "Redmine project").children.map((n) => n.value), ["all", "5", "4"]);
rmProject.value = "project2"; await rmProject.fire("change");
assert.equal(rmSource.value, "connection");
assert.equal(calls.at(-1).path, "/api/integration/redmine/connection/discover?project_id=project2");
assert.deepEqual(find(region, (n) => n["aria-label"] === "Redmine project").children.map((n) => n.value), ["all", "4", "5"]);
providers = [{ id: "github", name: "GitHub", sources: [
  { id: "owner/repo|issues", name: "owner/repo — Issues" },
  { id: "owner/repo|review", name: "owner/repo — PRs to review" }
], filters: [{ id: "user", label: "GitHub username" }] }];
providersByProject = { project: providers, project2: [{ id: "github", name: "GitHub", sources: [
  { id: "other/repo|follow", name: "other/repo — PRs to follow" }
], filters: [{ id: "user", label: "GitHub username" }] }] };
await renderImports(region, [{ id: "project", key: "TST", name: "Test" }, { id: "project2", key: "TWO", name: "Other" }], () => true, () => {});
const ghProject = find(region, (n) => n["aria-label"] === "Destination project");
const ghProvider = find(region, (n) => n["aria-label"] === "Provider");
const ghSource = find(region, (n) => n["aria-label"] === "Source");
const ghCategory = find(region, (n) => n["aria-label"] === "GitHub category");
const ghPreview = find(region, (n) => n.textContent === "Preview" && n.tag === "button");
ghProject.value = "project"; await ghProject.fire("change");
ghProvider.value = "github"; await ghProvider.fire("change");
ghSource.value = "owner/repo"; await ghSource.fire("change");
ghCategory.value = "issues"; await ghCategory.fire("change");
await ghPreview.fire("click");
assert.deepEqual(calls.at(-1).payload.filters, {});
assert.equal(calls.at(-1).payload.source, "owner/repo|issues");
const all = find(region, (n) => n["aria-label"] === "All assignees");
all.checked = true; await all.fire("change");
assert.equal(find(region, (n) => n["aria-label"] === "GitHub username").disabled, true);
await ghPreview.fire("click");
assert.deepEqual(calls.at(-1).payload.filters, { user: "" });
all.checked = false; await all.fire("change");
const ghUser = find(region, (n) => n["aria-label"] === "GitHub username");
assert.equal(ghUser.disabled, false);
ghUser.value = "alice"; await ghUser.fire("input");
await ghPreview.fire("click");
assert.deepEqual(calls.at(-1).payload.filters, { user: "alice" });
let releaseSwitch;
pendingProviders = new Promise((resolve) => { releaseSwitch = resolve; });
ghProject.value = "project2";
const switching = ghProject.fire("change");
assert.equal(ghSource.value, "");
assert.equal(ghSource.children.length, 1);
assert.equal(ghCategory.value, "");
assert.equal(ghCategory.children.length, 1);
assert.equal(find(region, (n) => n["aria-label"] === "GitHub username"), undefined);
assert.equal(ghPreview.disabled, true);
releaseSwitch(); await switching; pendingProviders = null;
assert.ok(ghSource.children.some((option) => option.value === "other/repo"));
assert.ok(!ghSource.children.some((option) => option.value === "owner/repo"));
ghSource.value = "other/repo"; await ghSource.fire("change");
assert.ok(ghCategory.children.some((option) => option.value === "follow"));
ghProject.value = ""; await ghProject.fire("change");
assert.equal(ghProvider.value, "");
assert.equal(ghSource.value, "");
assert.equal(ghSource.children.length, 1);
assert.equal(ghCategory.value, "");
assert.equal(ghCategory.children.length, 1);
assert.equal(ghPreview.disabled, true);
assert.equal(find(region, (n) => n["aria-label"] === "GitHub username"), undefined);
assert.equal(find(region, (n) => n.textContent === "Import selected"), undefined);
let releaseProviders;
pendingProviders = new Promise((resolve) => { releaseProviders = resolve; });
ghProject.value = "project";
const staleDiscovery = ghProject.fire("change");
ghProject.value = ""; await ghProject.fire("change");
releaseProviders(); await staleDiscovery; pendingProviders = null;
ghProvider.value = "github"; await ghProvider.fire("change");
assert.equal(ghSource.children.length, 1);
assert.equal(ghCategory.children.length, 1);
assert.equal(ghPreview.disabled, true);
console.log("imports UI workflow passed");

const { renderRedmineProjects } = await import("../web/redmine-projects.js");
let savedAssociations = [{ connection_id: "one", redmine_project_id: 99 }, { connection_id: "two", redmine_project_id: 8 }];
let savedPayload;
let discoveryFailure = false;
let pendingDiscovery;
let reappeared = false;
globalThis.fetch = async (path, options) => {
  let body;
  if (path === "/api/integration/redmine") body = [
    { id: "one", name: "First server", url: "https://one.example" },
    { id: "two", name: "Second server", url: "https://two.example" }
  ];
  else if (path.endsWith("/redmine-projects")) {
    if (options.method === "PUT") { savedPayload = JSON.parse(options.body); savedAssociations = savedPayload; }
    body = savedAssociations;
  } else {
    if (pendingDiscovery) await pendingDiscovery;
    if (discoveryFailure) return { ok: false, status: 503, headers: { get: () => "application/json" }, json: async () => ({ error: { message: "Discovery failed" } }) };
    body = { projects: path.includes("/one/") ? [{ id: 4, name: "<unsafe name>" }, { id: 5, name: "Beacon" }, ...(reappeared ? [{ id: 99, name: "Reappeared project" }] : [])] : [{ id: 8, name: "Other server project" }] };
  }
  return { ok: true, status: 200, headers: { get: () => "application/json" }, json: async () => body };
};
const settingsPanel = new FakeNode("div");
await renderRedmineProjects(settingsPanel, "project", () => true, () => {});
const settingConnection = find(settingsPanel, (n) => n["aria-label"] === "Redmine connection");
const settingSave = find(settingsPanel, (n) => n.textContent === "Save Redmine projects");
const unavailable = find(settingsPanel, (n) => n["aria-label"] === "Project #99 (currently unavailable)");
assert.equal(unavailable.checked, true);
assert.ok(find(settingsPanel, (n) => n["aria-label"] === "<unsafe name>"), "remote names are rendered as text");
const beacon = find(settingsPanel, (n) => n["aria-label"] === "Beacon");
beacon.checked = true; await beacon.fire("change");
settingConnection.value = "two"; await settingConnection.fire("change");
assert.equal(find(settingsPanel, (n) => n["aria-label"] === "Other server project").checked, true);
await settingSave.fire("click");
assert.deepEqual(savedPayload, [
  { connection_id: "one", redmine_project_id: 99, connection_url: "https://one.example" },
  { connection_id: "one", redmine_project_id: 5, connection_url: "https://one.example" },
  { connection_id: "two", redmine_project_id: 8, connection_url: "https://two.example" }
]);
settingConnection.value = "one"; await settingConnection.fire("change");
assert.equal(find(settingsPanel, (n) => n["aria-label"] === "Beacon").checked, true);
reappeared = true;
await settingConnection.fire("change");
assert.equal(find(settingsPanel, (n) => n["aria-label"] === "Reappeared project").checked, true);
reappeared = false;
await settingConnection.fire("change");
const stale = find(settingsPanel, (n) => n["aria-label"] === "Project #99 (currently unavailable)");
stale.checked = false; await stale.fire("change");
await settingSave.fire("click");
assert.ok(!savedPayload.some((a) => a.redmine_project_id === 99));
// A discovery failure cannot silently erase the existing selection.
discoveryFailure = true;
settingConnection.value = "two"; await settingConnection.fire("change");
assert.ok(find(settingsPanel, (n) => n.textContent === "Discovery failed"));
await settingSave.fire("click");
assert.ok(savedPayload.some((a) => a.connection_id === "two" && a.redmine_project_id === 8));
discoveryFailure = false;
// Only the latest connection's results may render; save is disabled in flight.
let releaseDiscovery;
pendingDiscovery = new Promise((resolve) => { releaseDiscovery = resolve; });
settingConnection.value = "one";
const oldLoad = settingConnection.fire("change");
assert.equal(settingSave.disabled, true);
settingConnection.value = "two";
const newLoad = settingConnection.fire("change");
releaseDiscovery();
await Promise.all([oldLoad, newLoad]);
pendingDiscovery = null;
assert.ok(find(settingsPanel, (n) => n["aria-label"] === "Other server project"));
assert.equal(find(settingsPanel, (n) => n["aria-label"] === "Beacon"), undefined);
console.log("Redmine project settings workflow passed");
