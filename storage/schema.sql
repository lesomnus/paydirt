CREATE TABLE IF NOT EXISTS paydirt_revisions (
    tenant BYTEA PRIMARY KEY NOT NULL,
    revision BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS paydirt_robots (
    id BYTEA PRIMARY KEY NOT NULL,
    tenant BYTEA NOT NULL,
    alias TEXT NOT NULL,
    created_ms BIGINT NOT NULL,
    updated_ms BIGINT NOT NULL,
    erased BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE UNIQUE INDEX IF NOT EXISTS paydirt_robot_slug ON paydirt_robots (tenant, alias) WHERE NOT erased;
CREATE INDEX IF NOT EXISTS paydirt_robot_page ON paydirt_robots (tenant, created_ms, id) WHERE NOT erased;
CREATE TABLE IF NOT EXISTS paydirt_changes (
    tenant BYTEA NOT NULL,
    revision BIGINT NOT NULL,
    robot_id BYTEA NOT NULL REFERENCES paydirt_robots(id),
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant, revision)
);
CREATE INDEX IF NOT EXISTS paydirt_watch ON paydirt_changes (tenant, robot_id, revision);
