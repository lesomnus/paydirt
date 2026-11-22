import assert from "node:assert/strict";
import {execFileSync} from "node:child_process";
import {randomBytes,generateKeyPairSync} from "node:crypto";
import {mkdtemp,writeFile,rm} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";

const cwd = await mkdtemp(join(tmpdir(),"paydirt-scan-"));
const env = {...process.env,DATABASE_URL:join(cwd,"db"),PAYDIRT_AUDIT_KEY:randomBytes(32).toString("hex"),
  PAYDIRT_ARCHIVE_DIR:join(cwd,"archive"), PAYDIRT_ARCHIVE_KEY:randomBytes(32).toString("hex"),
  PAYDIRT_SIGNING_KEY:join(cwd,"private.der"),PAYDIRT_WITNESS_DIR:join(cwd,"witness")};
const run = (...args) => JSON.parse(execFileSync(process.env.GENERATED_APP,args,{cwd,env,encoding:"utf8",stdio:["ignore","pipe","pipe"]}));
try {
  await writeFile(join(cwd,"app.json"),JSON.stringify({listen:"127.0.0.1:0",service_name:"scan",watch:{broker:"none"}}));
  await writeFile(env.PAYDIRT_SIGNING_KEY,generateKeyPairSync("ed25519").privateKey.export({type:"pkcs8",format:"der"}));
  const id = randomBytes(16); id[6]=128; id[8]=128; id[9]=1;
  const uuid=id.toString("hex").replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/,"$1-$2-$3-$4-$5");
  run("app.Tenant","Add",JSON.stringify({id:id.toString("base64"),alias:"owner"}));
  // Import a pre-trail Cell directly into the fixture, as an earlier deployment would.
  const cell=Buffer.from(id);cell[9]=10;
  execFileSync("python3",["-c",`import sqlite3,sys
with sqlite3.connect(sys.argv[1]) as db:
    db.execute('INSERT INTO pd_app__Cell (f1,f2,f4) VALUES (?,?,?)',(bytes.fromhex(sys.argv[2]),bytes.fromhex(sys.argv[3]),'scan'))`,env.DATABASE_URL,cell.toString("hex"),id.toString("hex")]);
  const cellId=cell.toString("hex").replace(/^(.{8})(.{4})(.{4})(.{4})(.{12})$/,"$1-$2-$3-$4-$5");
  assert.equal(run("trail-scan","app.Cell").findings[cellId].digest,"not_on_trail");
  assert.equal(run("trail-record","app.Cell",JSON.stringify({ids:[cellId,cellId],action:"/ops/Reconcile"})),1);
  assert.equal(run("trail-row","app.Cell",cellId).digest,"matches");
  run("trail",JSON.stringify({tenant:uuid,action:"policy",policy:{by:{cell:{retain_ms:0}}}}));
  assert.equal(run("trail",JSON.stringify({tenant:uuid,action:"collect"})).archived.length,1);
  const review=run("trail-check","review");
  assert.equal(review.database_checkpoint,1);assert.equal(review.archive_checkpoint,1);
  assert.equal(run("trail-check","verify").k,1);
  assert.throws(()=>run("trail-check",JSON.stringify({of:review.of,by:"operator",why:"tested",restored:1})));
  delete env.PAYDIRT_ARCHIVE_KEY;
  const report=run("trail-scan","app.Cell");
  assert.equal(report.archived,1);assert.deepEqual(report.findings,{});
  assert.equal(run("trail-row","app.Cell",cellId).digest,"matches");
} finally {await rm(cwd,{recursive:true,force:true});}
