CREATE TABLE issue_time_entries (
  entry_id UUID PRIMARY KEY,
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  issue_id UUID NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK (source IN ('opencode', 'manual')),
  kind TEXT NOT NULL CHECK (kind IN ('active_interval', 'manual_adjustment')),
  started_at TIMESTAMPTZ,
  ended_at TIMESTAMPTZ,
  duration_ms BIGINT NOT NULL CHECK (duration_ms <> 0),
  source_session_id TEXT,
  note TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  payload_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_issue_time_entries_project_issue
  ON issue_time_entries(project_id, issue_id);

CREATE INDEX idx_issue_time_entries_issue_id
  ON issue_time_entries(issue_id);

CREATE INDEX idx_issue_time_entries_user_received
  ON issue_time_entries(user_id, received_at DESC);

CREATE INDEX idx_issue_time_entries_source_kind
  ON issue_time_entries(source, kind);

ALTER TABLE issue_time_entries
  ADD CONSTRAINT issue_time_entries_source_kind_valid
    CHECK (
      (source = 'opencode' AND kind = 'active_interval')
      OR
      (source = 'manual' AND kind = 'manual_adjustment')
    ),
  ADD CONSTRAINT issue_time_entries_active_interval_valid
    CHECK (
      kind <> 'active_interval'
      OR (
        started_at IS NOT NULL
        AND ended_at IS NOT NULL
        AND started_at < ended_at
        AND duration_ms > 0
      )
    );

CREATE TABLE issue_time_totals (
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  issue_id UUID NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
  opencode_active_ms BIGINT NOT NULL DEFAULT 0,
  manual_adjustment_ms BIGINT NOT NULL DEFAULT 0,
  total_ms BIGINT NOT NULL DEFAULT 0,
  entry_count BIGINT NOT NULL DEFAULT 0,
  last_entry_at TIMESTAMPTZ,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, issue_id)
);

CREATE INDEX idx_issue_time_totals_project_id
  ON issue_time_totals(project_id);

CREATE INDEX idx_issue_time_totals_issue_id
  ON issue_time_totals(issue_id);

ALTER TABLE issue_time_totals
  ADD CONSTRAINT issue_time_totals_opencode_active_nonnegative
    CHECK (opencode_active_ms >= 0),
  ADD CONSTRAINT issue_time_totals_entry_count_nonnegative
    CHECK (entry_count >= 0),
  ADD CONSTRAINT issue_time_totals_total_matches_parts
    CHECK (total_ms = opencode_active_ms + manual_adjustment_ms),
  ADD CONSTRAINT issue_time_totals_total_nonnegative
    CHECK (total_ms >= 0);

CREATE OR REPLACE FUNCTION enforce_issue_time_issue_project()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM issues
    WHERE id = NEW.issue_id
      AND project_id = NEW.project_id
  ) THEN
    RAISE EXCEPTION 'issue_id does not belong to project_id'
      USING ERRCODE = 'foreign_key_violation';
  END IF;

  RETURN NEW;
END;
$$;

CREATE TRIGGER trg_issue_time_entries_issue_project
BEFORE INSERT OR UPDATE OF project_id, issue_id ON issue_time_entries
FOR EACH ROW
EXECUTE FUNCTION enforce_issue_time_issue_project();

CREATE TRIGGER trg_issue_time_totals_issue_project
BEFORE INSERT OR UPDATE OF project_id, issue_id ON issue_time_totals
FOR EACH ROW
EXECUTE FUNCTION enforce_issue_time_issue_project();

ALTER TABLE issue_time_totals REPLICA IDENTITY FULL;
SELECT electric_sync_table('public', 'issue_time_totals');
