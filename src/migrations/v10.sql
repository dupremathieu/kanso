CREATE TABLE task_reference_aliases (
  project_key TEXT NOT NULL COLLATE NOCASE,
  number INTEGER NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id),
  task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
  PRIMARY KEY (project_key, number)
);
