import { api } from "./api.js?v={{VERSION}}";
import { el } from "./dom.js?v={{VERSION}}";
import { t } from "./i18n/index.js?v={{VERSION}}";

export function redmineImportControls(invalidate) {
  const root = el("div");
  let version = 0;
  let ready = false;
  const project = el("select", { class: "input", "aria-label": t("redmine.project"), onChange: invalidate });
  const assignee = el("select", { class: "input", "aria-label": t("redmine.assignee"), onChange: change });
  const userId = el("input", { class: "input", type: "number", min: "1", step: "1", "aria-label": t("redmine.userId"), onInput: invalidate });
  const state = el("select", { class: "input", "aria-label": t("redmine.state"), onChange: invalidate });
  const watched = el("select", { class: "input", "aria-label": t("redmine.watched"), onChange: change });
  const operator = el("select", { class: "input", "aria-label": t("redmine.operator"), onChange: invalidate });
  const field = (label, control) => el("label", { class: "field" }, [el("span", { class: "field-label", text: label }), control]);
  const userField = field(t("redmine.userId"), userId);
  const operatorField = field(t("redmine.operator"), operator);
  const account = el("p", { class: "field-hint" });
  const error = el("p", { class: "form-error", role: "alert" });
  function option(value, label) { return el("option", { value, text: label }); }
  assignee.append(option("all", t("redmine.all")), option("me", t("redmine.me")), option("user", t("redmine.specifiedUser")));
  state.append(option("open", t("redmine.open")), option("closed", t("redmine.closed")), option("all", t("redmine.all")));
  watched.append(option("all", t("redmine.all")), option("me", t("redmine.watchedMe")));
  operator.append(option("and", t("redmine.and")), option("or", t("redmine.or")));
  function change() {
    userField.hidden = assignee.value !== "user";
    operatorField.hidden = assignee.value === "all" || watched.value === "all";
    invalidate();
  }
  root.append(account, field(t("redmine.project"), project), field(t("redmine.assignee"), assignee), userField,
    field(t("redmine.state"), state), field(t("redmine.watched"), watched), operatorField, error);
  change();
  async function load(id) {
    const next = ++version;
    ready = false;
    project.replaceChildren(option("", t("redmine.loading")));
    account.textContent = "";
    error.textContent = "";
    invalidate();
    if (!id) return;
    try {
      const data = await api.redmineDiscover(id);
      if (next !== version) return;
      project.replaceChildren(option("all", t("redmine.allProjects")), ...data.projects.map((p) => option(String(p.id), p.name)));
      account.textContent = t("redmine.account", { name: data.account.name });
      ready = true;
    } catch (failure) {
      if (next !== version) return;
      error.textContent = failure.message;
    }
    invalidate();
  }
  function valid() {
    return ready && (assignee.value !== "user" || /^[1-9]\d*$/.test(userId.value));
  }
  function values() {
    return { project: project.value, assignee: assignee.value === "user" ? userId.value : assignee.value,
      state: state.value, watched: watched.value, operator: operator.value };
  }
  return { root, load, valid, values };
}
