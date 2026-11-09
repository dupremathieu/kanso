-- Preserve task IDs and all referencing rows while extending the status check.
CREATE TABLE tasks_new (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  number INTEGER NOT NULL,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL CHECK (status IN ('backlog','todo','in_progress','in_review','blocker','done','dropped')),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','proposed','archived')),
  blocked_from TEXT,
  blocked_reason TEXT,
  rank REAL NOT NULL,
  priority TEXT NOT NULL DEFAULT 'normal' CHECK (priority IN ('low','normal','high')),
  due_date TEXT,
  source TEXT NOT NULL DEFAULT 'human' CHECK (source IN ('human','agent')),
  created_by TEXT,
  claimed_by TEXT,
  session_id TEXT,
  session_directory TEXT,
  session_status TEXT,
  attention INTEGER NOT NULL DEFAULT 0,
  done_at INTEGER,
  archived_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (project_id, number)
);
INSERT INTO tasks_new SELECT * FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;
CREATE INDEX tasks_board ON tasks (project_id, state, status, rank);
CREATE INDEX tasks_session ON tasks (session_id) WHERE session_id IS NOT NULL;
CREATE TRIGGER secret_task_invalidate AFTER UPDATE OF status, state, session_id, session_directory, blocked_reason ON tasks
WHEN OLD.status IS NOT NEW.status OR OLD.state IS NOT NEW.state OR OLD.session_id IS NOT NEW.session_id OR OLD.session_directory IS NOT NEW.session_directory OR OLD.blocked_reason IS NOT NEW.blocked_reason
BEGIN
  UPDATE secret_requests SET state = 'invalidated', verifier = NULL,
    invalidation_reason = CASE WHEN OLD.session_id IS NOT NEW.session_id OR OLD.session_directory IS NOT NEW.session_directory THEN 'session_changed' ELSE 'card_changed' END
  WHERE task_id = NEW.id AND state IN ('waiting','ready')
    AND (NEW.status <> 'blocker' OR NEW.state <> 'active' OR NEW.session_id IS NOT session_id OR NEW.session_directory IS NOT session_directory OR OLD.blocked_reason IS NOT NEW.blocked_reason);
  UPDATE secret_requests SET invalidation_reason = 'session_changed'
  WHERE task_id = NEW.id AND (OLD.session_id IS NOT NEW.session_id OR OLD.session_directory IS NOT NEW.session_directory);
END;
