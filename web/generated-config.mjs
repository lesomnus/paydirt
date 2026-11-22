import assert from "node:assert/strict";
import {execFileSync} from "node:child_process";
import {mkdtemp, rm, writeFile} from "node:fs/promises";
import {tmpdir} from "node:os";
import {join} from "node:path";

const cwd = await mkdtemp(join(tmpdir(), "paydirt-env-"));
const env = {...process.env, DATABASE_URL:"invalid://never-open", DEPLOY_BEARER:"a-private-token"};
delete env.PAYDIRT_LISTEN;
const run = (...args) => execFileSync(process.env.GENERATED_APP, args, {cwd,env,encoding:"utf8",stdio:["ignore","pipe","pipe"]}).trim().split("\n");
try {
  await writeFile(join(cwd,"app.json"), JSON.stringify({
    watch:{broker:"none"}, listen:"127.0.0.1:0", service_name:"config-test",
    authentication:{introspection:{url:"http://unused.invalid",bearer_env:"DEPLOY_BEARER"}},
    grant_source:{url:"http://unused.invalid",bearer_env:"GRANT_TOKEN"},
  }));
  assert(run("--help")[0].split(" ").includes("config"));
  const names = run("config","env");
  assert.deepEqual(names, [...new Set(names)].sort());
  for (const name of ["DATABASE_URL","PAYDIRT_LISTEN","PAYDIRT_WATCH_BROKER","PAYDIRT_WORKER_LEASE","DEPLOY_BEARER","GRANT_TOKEN"]) assert(names.includes(name));
  const state = run("config","env","--set");
  assert.deepEqual(state, names.map(k => k+"\t"+(Object.hasOwn(env,k)?"set":"-")));
  assert.throws(()=>run("config","env","invalid"));
} finally {
  await rm(cwd,{recursive:true,force:true});
}
