CREATE TABLE clients (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    archived_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT NOT NULL,
    client_id TEXT REFERENCES clients(id) ON DELETE RESTRICT,
    is_billable INTEGER NOT NULL,
    billable_rate INTEGER,
    estimated_time INTEGER,
    archived_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    estimated_time INTEGER,
    done_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE timer_sessions (
    id TEXT PRIMARY KEY,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    state TEXT NOT NULL CHECK (state IN ('running', 'paused', 'stopped')),
    template_json TEXT
);

CREATE TABLE time_entries (
    id TEXT PRIMARY KEY,
    session_id TEXT REFERENCES timer_sessions(id) ON DELETE SET NULL,
    description TEXT NOT NULL,
    start INTEGER NOT NULL,
    "end" INTEGER,
    billable INTEGER NOT NULL,
    project_id TEXT REFERENCES projects(id) ON DELETE RESTRICT,
    task_id TEXT REFERENCES tasks(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_time_entries_start ON time_entries(start);
CREATE INDEX idx_time_entries_session ON time_entries(session_id);
CREATE INDEX idx_time_entries_project ON time_entries(project_id);
CREATE INDEX idx_time_entries_task ON time_entries(task_id);

-- At most one running entry: every row with "end" IS NULL maps to the same key.
CREATE UNIQUE INDEX one_running ON time_entries("end" IS NULL) WHERE "end" IS NULL;

CREATE TABLE time_entry_tags (
    time_entry_id TEXT NOT NULL REFERENCES time_entries(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (time_entry_id, tag_id)
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
