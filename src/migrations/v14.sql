ALTER TABLE projects ADD COLUMN parent_id TEXT REFERENCES projects(id);

CREATE INDEX projects_parent_idx ON projects(parent_id, archived_at);
