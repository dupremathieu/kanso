import { api } from "./api.js?v={{VERSION}}";
import { el } from "./dom.js?v={{VERSION}}";
import { t } from "./i18n/index.js?v={{VERSION}}";

export async function renderRedmineProjects(panel, projectId, current, setSaving) {
  const [connections, initial] = await Promise.all([api.redmineConnections(), api.redmineProjects(projectId)]);
  if (!current()) return;
  const selected = new Map(connections.map((c) => [c.id, new Set(initial.filter((a) => a.connection_id === c.id).map((a) => a.redmine_project_id))]));
  let generation = 0;
  const connection = el("select", { class: "input", "aria-label": t("redmine.connection") },
    connections.map((c) => el("option", { value: c.id, text: c.name })));
  const list = el("div", { class: "redmine-project-list" });
  const error = el("p", { class: "form-error", role: "alert" });
  const status = el("p", { role: "status", class: "field-hint" });
  const save = el("button", { type: "button", class: "button button-primary button-small", text: t("redmine.saveProjects"), disabled: !connections.length });
  async function load() {
    const next = ++generation;
    save.disabled = true;
    list.replaceChildren(el("p", { text: t("redmine.loading") }));
    error.textContent = "";
    status.textContent = "";
    if (!connection.value) { list.replaceChildren(); return; }
    const id = connection.value;
    try {
      const data = await api.redmineDiscover(id);
      if (!current() || next !== generation) return;
      const choices = [...data.projects];
      const available = new Set(choices.map((p) => p.id));
      // Keep unavailable IDs visible and removable; never drop them on save.
      for (const saved of selected.get(id)) {
        if (!available.has(saved)) choices.push({ id: saved, name: t("redmine.unavailableProject", { id: saved }) });
      }
      list.replaceChildren(...choices.map((p) => {
        const checkbox = el("input", { type: "checkbox", checked: selected.get(id).has(p.id), "aria-label": p.name,
          onChange: () => { if (checkbox.checked) selected.get(id).add(p.id); else selected.get(id).delete(p.id); status.textContent = ""; } });
        return el("label", { class: "field-hint" }, [checkbox, ` ${p.name}`]);
      }));
      if (!choices.length) list.append(el("p", { class: "field-hint", text: t("redmine.noProjects") }));
    } catch (failure) {
      if (!current() || next !== generation) return;
      list.replaceChildren();
      error.textContent = failure.message;
    } finally {
      if (current() && next === generation) save.disabled = false;
    }
  }
  connection.addEventListener("change", load);
  save.addEventListener("click", async () => {
    save.disabled = true;
    connection.disabled = true;
    list.querySelectorAll("input").forEach((input) => { input.disabled = true; });
    setSaving(true);
    error.textContent = "";
    status.textContent = "";
    try {
      const associations = [...selected].flatMap(([connection_id, ids]) => [...ids].map((redmine_project_id) => ({
        connection_id, redmine_project_id, connection_url: connections.find((c) => c.id === connection_id).url
      })));
      await api.saveRedmineProjects(projectId, associations);
      if (current()) status.textContent = t("redmine.projectsSaved");
    } catch (failure) { if (current()) error.textContent = failure.message; }
    finally {
      save.disabled = false;
      connection.disabled = false;
      list.querySelectorAll("input").forEach((input) => { input.disabled = false; });
      setSaving(false);
    }
  });
  panel.replaceChildren(el("h3", { text: t("redmine.projects") }),
    el("p", { class: "field-hint", text: t("redmine.projectsHint") }),
    ...(connections.length ? [el("label", { class: "field" }, [el("span", { class: "field-label", text: t("redmine.connection") }), connection])]
      : [el("p", { class: "field-hint", text: t("redmine.noConnections") })]),
    list, save, status, error);
  await load();
}
