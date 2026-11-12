-- Recall prompts are separate from blocker replies, so neither retry endpoint
-- can accidentally forward the other kind of comment.
ALTER TABLE comments ADD COLUMN delivery_kind TEXT;
ALTER TABLE tasks ADD COLUMN recall_revision INTEGER NOT NULL DEFAULT 0;
CREATE TRIGGER task_recall_revision AFTER UPDATE ON tasks
WHEN NEW.recall_revision = OLD.recall_revision
BEGIN
  UPDATE tasks SET recall_revision = OLD.recall_revision + 1 WHERE id = NEW.id;
END;
CREATE TABLE recall_deliveries (
  comment_id TEXT PRIMARY KEY REFERENCES comments(id) ON DELETE CASCADE,
  session_id TEXT NOT NULL,
  session_directory TEXT,
  prompt TEXT NOT NULL
);
