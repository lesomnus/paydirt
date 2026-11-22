CREATE TABLE "pd_app__Access" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f2" BLOB NOT NULL,
  "f8" BLOB NOT NULL,
  "f9" BLOB NOT NULL,
  "f10" TEXT NOT NULL,
  "f11" TEXT NOT NULL,
  "f12" TEXT NOT NULL,
  "f15" TEXT NOT NULL,
  "f16" BLOB NOT NULL,
  "f17" BLOB NOT NULL,
  "f19" BIGINT NOT NULL,
  "f20" TEXT NOT NULL,
  "f21" BIGINT NOT NULL
);
CREATE TABLE "pd_app__Archived" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f7" TEXT NOT NULL,
  "f8" TEXT NOT NULL,
  "f9" TEXT NOT NULL,
  "f10" TEXT NOT NULL,
  "f11" TEXT NOT NULL,
  "f12" BIGINT NOT NULL,
  "f13" TEXT NOT NULL,
  "f15" TEXT NOT NULL,
  "f16" BIGINT NOT NULL,
  "f17" BLOB NOT NULL,
  "f18" BLOB NOT NULL
);
CREATE TABLE "pd_app__Audit" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f2" BLOB NOT NULL,
  "f8" BLOB NOT NULL,
  "f9" BLOB NOT NULL,
  "f10" TEXT NOT NULL,
  "f11" BLOB NOT NULL,
  "f12" BLOB NOT NULL,
  "f15" TEXT NOT NULL,
  "f16" BLOB NOT NULL,
  "f17" BLOB NOT NULL,
  "f18" BLOB,
  "f19" BIGINT NOT NULL,
  "f20" TEXT NOT NULL,
  "f21" BLOB NOT NULL,
  "f22" TEXT NOT NULL,
  "f23" BLOB,
  "f24" BLOB,
  "f25" TEXT
);
CREATE TABLE "pd_app__Cell" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f2" BLOB NOT NULL REFERENCES "pd_app__Tenant" ("f1"),
  "f4" TEXT NOT NULL,
  "f14" TEXT
);
CREATE TABLE "pd_app__Holder" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f2" BLOB NOT NULL REFERENCES "pd_app__Tenant" ("f1"),
  "f4" TEXT NOT NULL,
  "f5" TEXT NOT NULL,
  "f6" TEXT NOT NULL,
  "f7" TEXT NOT NULL,
  "f13" TEXT NOT NULL,
  "f14" TEXT,
  "f15" TEXT NOT NULL
);
CREATE TABLE "pd_app__Outbox" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f2" BLOB NOT NULL,
  "f8" BLOB NOT NULL,
  "f9" TEXT NOT NULL,
  "f10" TEXT NOT NULL,
  "f11" BLOB NOT NULL,
  "f12" BLOB NOT NULL,
  "f15" TEXT NOT NULL,
  "f16" BLOB,
  "f17" BLOB,
  "f18" BLOB
);
CREATE TABLE "pd_app__Owed" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f8" BLOB NOT NULL,
  "f9" TEXT NOT NULL,
  "f15" TEXT NOT NULL
);
CREATE TABLE "pd_app__Robot" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f2" BLOB NOT NULL REFERENCES "pd_app__Tenant" ("f1"),
  "f3" BLOB REFERENCES "pd_app__Cell" ("f1"),
  "f4" TEXT NOT NULL,
  "f13" TEXT NOT NULL,
  "f14" TEXT,
  "f15" TEXT NOT NULL
);
CREATE TABLE "pd_app__RowSeal" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f2" BLOB NOT NULL,
  "f8" BLOB NOT NULL,
  "f9" TEXT NOT NULL
);
CREATE TABLE "pd_app__Tenant" (
  "f1" BLOB NOT NULL PRIMARY KEY,
  "f4" TEXT NOT NULL,
  "f5" TEXT NOT NULL,
  "f6" TEXT NOT NULL,
  "f7" TEXT NOT NULL,
  "f13" TEXT NOT NULL,
  "f15" TEXT NOT NULL
);
CREATE INDEX "pd_app__Access__access" ON "pd_app__Access" ("f2", "f15");
CREATE INDEX "pd_app__Access__by_actor" ON "pd_app__Access" ("f8", "f15");
CREATE INDEX "pd_app__Access__by_actor_tenant" ON "pd_app__Access" ("f16", "f15");
CREATE INDEX "pd_app__Access__by_time" ON "pd_app__Access" ("f15", "f1");
CREATE INDEX "pd_app__Archived__blob" ON "pd_app__Archived" ("f8", "f9");
CREATE INDEX "pd_app__Archived__state" ON "pd_app__Archived" ("f11");
CREATE INDEX "pd_app__Audit__by_actor" ON "pd_app__Audit" ("f8", "f15");
CREATE INDEX "pd_app__Audit__by_actor_tenant" ON "pd_app__Audit" ("f16", "f15");
CREATE INDEX "pd_app__Audit__by_counterpart" ON "pd_app__Audit" ("f18", "f15");
CREATE INDEX "pd_app__Audit__by_domain" ON "pd_app__Audit" ("f19", "f15");
CREATE INDEX "pd_app__Audit__by_tenant_domain" ON "pd_app__Audit" ("f2", "f19", "f15");
CREATE INDEX "pd_app__Audit__by_time" ON "pd_app__Audit" ("f15", "f1");
CREATE INDEX "pd_app__Audit__object" ON "pd_app__Audit" ("f11");
CREATE INDEX "pd_app__Audit__trail" ON "pd_app__Audit" ("f2", "f15");
CREATE INDEX "pd_app__Holder__page" ON "pd_app__Holder" ("f2", "f15", "f1");
CREATE UNIQUE INDEX "pd_app__Holder__slug" ON "pd_app__Holder" ("f4", "f2") WHERE "f14" IS NULL;
CREATE INDEX "pd_app__Outbox__queue" ON "pd_app__Outbox" ("f1");
CREATE INDEX "pd_app__Owed__owed" ON "pd_app__Owed" ("f15", "f1");
CREATE INDEX "pd_app__Robot__by_alias" ON "pd_app__Robot" ("f2", "f4", "f1");
CREATE INDEX "pd_app__Robot__page" ON "pd_app__Robot" ("f2", "f15", "f1");
CREATE UNIQUE INDEX "pd_app__Robot__slug" ON "pd_app__Robot" ("f4", "f2") WHERE "f14" IS NULL;
CREATE INDEX "pd_app__Tenant__page" ON "pd_app__Tenant" ("f15", "f1");
CREATE UNIQUE INDEX "pd_app__Tenant__field_4" ON "pd_app__Tenant" ("f4");
