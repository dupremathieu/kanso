# KANSO-3 — Redmine issue importer

## Approval status

Product decisions below and this consolidated specification were explicitly
approved by the human on 2026-11-16. Implementation is authorized.

Implementation must be delegated to a dedicated OpenChamber session in the
current worktree. Use atomic commits and call the checker subagent before each
commit. The coordinating session verifies the result and opens the PR. Never
move KANSO-3 to done or modify other cards.

## Scope

- Manual, one-way import of Redmine issues into an explicitly selected existing,
  active Kanso project.
- No Redmine or Kanso project creation, project metadata import, synchronization,
  upstream writes, polling, webhooks, or deletion propagation.
- Status mapping is explicitly deferred despite the original card description.
  Imported tasks use existing defaults: backlog, active state, normal priority.
- Copy issue subject to title and description to description. Include a compact
  Redmine issue reference and clickable upstream link in the description; retain
  normal importer provenance and source-link display. Do not create a synthetic
  comment. Preserve source description text without promising Textile conversion.
- Do not import assignees, watchers, comments, labels, attachments, due dates,
  priorities, custom fields, or task hierarchy. Existing optional local import
  labels remain available.

## Connections and credentials

- Browser integration settings manage multiple named connections, each with a
  Redmine instance base URL and the account holder's personal API key.
- Connections are global to Kanso, consistent with its shared human login.
  All human browser sessions can use/manage them; this is not per-person
  credential isolation. Show the authenticated Redmine account when available.
- “Me” always refers to the Redmine account authenticated by the selected key.
- Persist connections atomically in a server-owned mode-0600 file outside
  SQLite, following the existing OpenChamber integration pattern.
- Keys are plaintext on disk. Explain this in settings and documentation.
  They are outside JSON exports and database-only backups, but included in full
  filesystem backups, which must be protected.
- Keys are write-only through browser settings. Never return an existing key.
  Never include keys in metadata, URLs, tasks, activity, SSE, logs, or errors.
  Use the X-Redmine-API-Key header, never a URL query parameter.
- Logout retains connections. Deleting a connection removes its saved key but
  does not revoke the key in Redmine.
- Connection management requires an authenticated browser session and existing
  CSRF protections. Import endpoints remain human-only; reject agent tokens.
- Accept safe HTTP(S) instance URLs, including host-local/private test instances
  and deployments under a path prefix. Reject embedded credentials and unsafe
  URL components. Warn that HTTP does not encrypt the API key in transit.
  Do not forward credentials through redirects to another origin.

## Import controls

- Choose destination Kanso project and named Redmine connection.
- Discover projects accessible with the selected key. Filter by all accessible
  projects or one project; a selected project excludes its subprojects.
- Assignee: all (default), me, or a specified Redmine user.
- State: open (default), closed, or all.
- Watched: unrestricted (default), or watched by me.
- Offer AND/OR only between active assignee and watched predicates:
  `project AND state AND (assignee AND/OR watched)`.
- If only one of assignee/watched is restricted, apply that predicate alone;
  an unrestricted predicate must not make OR match everything.
- If implementing OR using two upstream queries, apply project/state constraints
  to both, retrieve complete bounded results, and deduplicate by issue identity.
  Failure in either query fails preview rather than presenting a partial union.
- Do not silently ignore unsupported or failed filters. Typed controls should
  not require users to construct Redmine query syntax.

## Shared importer guarantees

- Reuse preview, explicit individual selection, immutable server-side snapshots,
  and per-item created/skipped/failed confirmation outcomes.
- Preview never creates tasks. Changing connection, destination, or filters
  invalidates prior selection. Confirmation cannot forge issue content.
- Bound requests, timeouts, pagination and aggregate response size. Retain shared
  preview limits (currently 200 issues and 2 MiB). Report oversized searches and
  require narrower filters; never silently truncate or disguise failure as empty.
- Deduplicate by stable Redmine instance identity plus numeric issue ID within
  the destination Kanso project, not by title, account, or connection name.
  Connections to the same canonical instance must not duplicate imports.
- Repeat imports skip existing cards, including archived cards, preserving local
  edits and status. Allow the same upstream issue in different Kanso projects.
- Preserve transactional task/provenance creation, normal activity/events,
  title/description validation, safe rendering and provenance exports/backups.
- Connection removal or replacement invalidates pending previews using it.
  Token rotation and connection renaming must not change upstream issue identity.
- Preserve all existing GitHub behavior and shared importer authorization rules.

## Verification and handoff

- Cover connection persistence, permissions, write-only responses, reload,
  rotation/deletion, account detection, authorization and CSRF.
- Cover project discovery, assignee/state/watch filters, both AND and OR,
  overlapping OR results, inactive-predicate behavior, pagination, bounds,
  timeouts and upstream errors.
- Cover preview/confirm, provenance and upstream links, normal defaults,
  deduplication across connections, local edit preservation, invalid content,
  changed/expired previews, and GitHub regression behavior.
- Run formatting, linting, relevant backend/frontend tests and browser checks;
  document commands and results.
- Validate against the disposable Redmine instance described on KANSO-3.
  Obtain credentials only via `kanso secret request` and consume them only using
  `kanso secret exec` with a trusted command. Never read existing secret files.
  Do not reset or mutate the Redmine fixture for this read-only integration.
- Update importer, integration setup and backup/security documentation.
- Capture the implemented UI and commit screenshots under
  `docs/screenshots/kanso-3/`. Include them in the PR description using absolute
  raw GitHub image URLs, as explicitly requested by the human.
- Coordinator verifies delegated work, opens a PR, moves KANSO-3 to in_review,
  and comments the implementation/test summary and PR link.
