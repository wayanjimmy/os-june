CREATE TABLE IF NOT EXISTS dictation_history (
  id TEXT PRIMARY KEY NOT NULL,
  text TEXT NOT NULL,
  language TEXT,
  provider TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dictation_history_created_at
  ON dictation_history (created_at DESC);
