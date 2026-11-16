import { api } from "./api.js?v={{VERSION}}";
import { el } from "./dom.js?v={{VERSION}}";
import { t } from "./i18n/index.js?v={{VERSION}}";

export function redmineSettings(current) {
  const root = el("section", { class: "settings-list", "aria-label": t("redmine.title") });
  let entries = [];
  let editing = null;
  let error = "";
  let busy = false;
  let generation = 0;
  const accounts = {};
  const input = (label, type = "text", value = "") => el("input", {
    class: "input", type, value, "aria-label": label, autocomplete: "off"
  });
  const field = (label, control) => el("label", { class: "field" }, [
    el("span", { class: "field-label", text: label }), control
  ]);
  async function refresh() {
    try { entries = await api.redmineConnections(); error = ""; }
    catch (failure) { error = failure.message; }
    if (current()) draw();
  }
  function draw() {
    const name = input(t("redmine.name"), "text", editing?.name || "");
    const url = input(t("redmine.url"), "url", editing?.url || "");
    const key = input(t("redmine.key"), "password");
    const save = el("button", { type: "submit", class: "button button-primary", text: t("redmine.save"), disabled: busy });
    const form = el("form", {}, [
      field(t("redmine.name"), name), field(t("redmine.url"), url), field(t("redmine.key"), key),
      el("p", { class: "field-hint", text: t("redmine.keyHint") }), save
    ]);
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      if (busy) return;
      busy = true;
      save.disabled = true;
      try {
        await api.saveRedmineConnection(editing?.id, { name: name.value, url: url.value, key: key.value || null });
        generation += 1;
        key.value = "";
        if (editing) delete accounts[editing.id];
        editing = null;
        busy = false;
        await refresh();
      } catch (failure) {
        error = failure.message;
        let alert = root.querySelector("[data-redmine-error]");
        if (!alert) {
          alert = el("p", { class: "form-error", role: "alert", "data-redmine-error": "" });
          form.after(alert);
        }
        alert.textContent = error;
      }
      finally { key.value = ""; busy = false; save.disabled = false; }
    });
    const rows = entries.map((entry) => el("article", { class: "settings-row" }, [
      el("div", { class: "settings-row-main" }, [
        el("strong", { text: entry.name }), el("p", { class: "field-hint", text: entry.url }),
        accounts[entry.id] ? el("p", { class: "field-hint", text: accounts[entry.id] }) : null
      ]),
      el("div", { class: "settings-row-actions" }, [
        el("button", { type: "button", class: "button button-secondary", text: t("redmine.checkAccount"), onClick: async () => {
          const version = generation;
          try { const account = await api.redmineAccount(entry.id); if (version === generation) accounts[entry.id] = t("redmine.account", { name: account.name }); }
          catch (failure) { if (version === generation) accounts[entry.id] = failure.message; }
          if (current() && version === generation) draw();
        } }),
        el("button", { type: "button", class: "button button-secondary", text: t("redmine.edit"), onClick: () => { editing = entry; error = ""; draw(); } }),
        el("button", { type: "button", class: "button button-secondary", text: t("redmine.delete"), onClick: async () => {
          if (!window.confirm(t("redmine.deleteConfirm"))) return;
          try { await api.deleteRedmineConnection(entry.id); generation += 1; delete accounts[entry.id]; if (editing?.id === entry.id) editing = null; await refresh(); }
          catch (failure) { error = failure.message; if (current()) draw(); }
        } })
      ])
    ]));
    root.replaceChildren(el("article", { class: "settings-row" }, [
      el("div", { class: "settings-row-main" }, [
        el("h2", { text: t("redmine.title") }),
        el("p", { class: "field-hint", text: t("redmine.warning") }),
        el("p", { class: "field-hint", text: t("redmine.httpWarning") }),
        ...rows,
        el("h3", { text: editing ? t("redmine.edit") : t("redmine.add") }),
        form,
        error ? el("p", { class: "form-error", role: "alert", text: error }) : null
      ])
    ]));
  }
  root.append(el("p", { class: "muted", role: "status", text: t("common.loading") }));
  refresh();
  return root;
}
