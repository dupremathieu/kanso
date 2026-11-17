CREATE TABLE redmine_projects (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    connection_id TEXT NOT NULL,
    connection_url TEXT NOT NULL,
    redmine_project_id INTEGER NOT NULL CHECK (redmine_project_id > 0),
    PRIMARY KEY (project_id, connection_id, redmine_project_id)
);
