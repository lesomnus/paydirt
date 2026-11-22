"""Exercise both generated SQL schemas against live engines in disposable databases."""
import os
from pathlib import Path
import sqlite3
import subprocess
import uuid

storage = Path(__file__).resolve().parents[1] / "generated/storage"
tenant = '''INSERT INTO "pd_app__Tenant" (f1,f4,f5,f6,f7,f13,f15) VALUES (X'01','one','','','{}','now','now');'''
robot = '''INSERT INTO "pd_app__Robot" (f1,f2,f4,f13,f15) VALUES (X'02',X'01','robot','now','now');'''
again = robot.replace("X'02'", "X'03'")
erase = '''UPDATE "pd_app__Robot" SET f14='now' WHERE f1=X'02';'''
orphan = '''INSERT INTO "pd_app__Cell" (f1,f2,f4) VALUES (X'05',X'ff','lost');'''

db = sqlite3.connect(":memory:")
db.execute("PRAGMA foreign_keys=ON")
db.executescript((storage / "sqlite.sql").read_text())
db.executescript(tenant + robot)
for statement in [again, orphan]:
    try:
        db.execute(statement)
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError(statement)
db.executescript(erase + again)
assert db.execute('SELECT count(*) FROM "pd_app__Robot"').fetchone() == (2,)

def pg(sql):
    import re
    return re.sub(r"X'([0-9a-f]+)'", r"decode('\1','hex')", sql)

schema = "schema_" + uuid.uuid4().hex
body = "BEGIN; CREATE SCHEMA " + schema + "; SET LOCAL search_path TO " + schema + ";\n"
body += (storage / "postgres.sql").read_text() + pg(tenant + robot)
for statement, error in [(again, "unique_violation"), (orphan, "foreign_key_violation")]:
    body += "DO $$ BEGIN " + pg(statement) + " RAISE EXCEPTION 'missing constraint'; EXCEPTION WHEN " + error + " THEN NULL; END $$;"
body += pg(erase + again) + "ROLLBACK;"
subprocess.run(["psql", os.environ["DATABASE_URL"], "-Xq", "-v", "ON_ERROR_STOP=1"], input=body, text=True, check=True)
print("PostgreSQL and SQLite enforce relationship keys, active-only uniqueness and reuse after soft erase.")
