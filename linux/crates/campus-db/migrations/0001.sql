CREATE TABLE account (id INTEGER PRIMARY KEY, student_id TEXT NOT NULL UNIQUE,
  name TEXT, identity TEXT, department TEXT, created_at INTEGER NOT NULL, last_login_at INTEGER);

CREATE TABLE http_session (                       -- cookie jars, always encrypted
  account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  jar TEXT NOT NULL CHECK (jar IN ('school','phys_lab','phys_demo','phyvlab')),
  cookies BLOB NOT NULL, nonce BLOB NOT NULL,     -- includes session-only cookies
  aa_linked INTEGER NOT NULL DEFAULT 0, smart_ready INTEGER NOT NULL DEFAULT 0,
  saved_at INTEGER NOT NULL, verified_at INTEGER, PRIMARY KEY (account_id, jar));
CREATE TABLE secret (name TEXT PRIMARY KEY, ciphertext BLOB NOT NULL, nonce BLOB NOT NULL); -- only without a keyring

CREATE TABLE course (account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL, course_code TEXT NOT NULL, name TEXT NOT NULL, teacher TEXT NOT NULL,
  location_index INTEGER NOT NULL,
  weekday INTEGER GENERATED ALWAYS AS (location_index % 8) VIRTUAL,
  slot    INTEGER GENERATED ALWAYS AS (location_index / 8) VIRTUAL,
  weeks_raw TEXT NOT NULL, weeks_mask INTEGER NOT NULL,   -- bit w = teaching week w
  place TEXT NOT NULL, is_selection INTEGER NOT NULL, PRIMARY KEY (account_id, ordinal));
CREATE INDEX course_day ON course(account_id, weekday, slot);

CREATE TABLE grade (account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL, course_name TEXT NOT NULL, course_code TEXT, teacher TEXT NOT NULL,
  term TEXT NOT NULL, score_raw TEXT NOT NULL, score_letter TEXT, score_value REAL,
  credits_raw TEXT NOT NULL, credits REAL, detail TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (account_id, ordinal));
CREATE INDEX grade_term ON grade(account_id, term);
CREATE TABLE grade_selection (account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  course_name TEXT NOT NULL, teacher TEXT NOT NULL, term TEXT NOT NULL, occurrence INTEGER NOT NULL,
  last_known_score TEXT NOT NULL, last_known_credits TEXT NOT NULL,
  PRIMARY KEY (account_id, course_name, teacher, term, occurrence));
CREATE TABLE program_course_type (account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  course_code TEXT NOT NULL,
  course_type TEXT NOT NULL CHECK (course_type IN ('required','limited','elective','pe')),
  PRIMARY KEY (account_id, course_code));

CREATE TABLE exam (account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL, exam_type TEXT NOT NULL, course_name TEXT NOT NULL,
  time_place_raw TEXT NOT NULL, status TEXT NOT NULL, detail TEXT NOT NULL,
  starts_at INTEGER, ends_at INTEGER, location TEXT, PRIMARY KEY (account_id, ordinal));
CREATE INDEX exam_start ON exam(account_id, starts_at);

CREATE TABLE card_status (account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  fetched_at INTEGER NOT NULL, card_balance TEXT NOT NULL, net_fee TEXT NOT NULL,
  new_mail INTEGER NOT NULL, PRIMARY KEY (account_id, fetched_at));   -- history, kept 90 days

CREATE TABLE academic_week (term TEXT NOT NULL, week INTEGER NOT NULL, monday TEXT NOT NULL,
  PRIMARY KEY (term, week));
CREATE TABLE semester_option (id TEXT PRIMARY KEY, label TEXT NOT NULL, is_current INTEGER NOT NULL);
CREATE TABLE room_occupancy (term TEXT NOT NULL, building_id TEXT NOT NULL, week INTEGER NOT NULL,
  room TEXT NOT NULL, capacity INTEGER NOT NULL,
  cells BLOB NOT NULL CHECK (length(cells)=49), fetched_at INTEGER NOT NULL,
  PRIMARY KEY (term, building_id, week, room));

-- phase 6: homework, courseware_course, courseware_node, download (resume checkpoint as a bitmap)

CREATE TABLE sync_state (account_id INTEGER NOT NULL, dataset TEXT NOT NULL,  -- account 0 = public data
  last_attempt_at INTEGER, last_success_at INTEGER,
  last_status TEXT CHECK (last_status IN ('success','failed','skipped','needs_user_action')),
  last_reason TEXT, PRIMARY KEY (account_id, dataset));
CREATE TABLE change_event (id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
  dataset TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind IN ('added','modified','deleted')),
  item_key TEXT NOT NULL, before_json TEXT, after_json TEXT,
  detected_at INTEGER NOT NULL, seen INTEGER NOT NULL DEFAULT 0);     -- kept to 100 per account
CREATE TABLE pref (scope TEXT NOT NULL DEFAULT '', key TEXT NOT NULL, value TEXT NOT NULL,
  PRIMARY KEY (scope, key));

CREATE TABLE dataset_row(account_id INTEGER NOT NULL, dataset TEXT NOT NULL, ordinal INTEGER NOT NULL, payload TEXT NOT NULL CHECK(json_valid(payload)), PRIMARY KEY(account_id,dataset,ordinal));
