ALTER TABLE labels ADD COLUMN is_global INTEGER NOT NULL DEFAULT 1 CHECK (is_global IN (0, 1));
CREATE TABLE label_projects (
    label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    PRIMARY KEY (label_id, project_id)
);
