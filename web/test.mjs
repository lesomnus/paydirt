import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { createServer } from "vite";
import { chromium } from "playwright";

function principal() {
  const id = randomBytes(16);
  id[6] = 0x80;
  id[8] = 0x80;
  id[9] = 1;
  return [randomBytes(24).toString("hex"), id.toString("hex")];
}

assert(process.env.DATABASE_URL, "DATABASE_URL must name a disposable PostgreSQL database");
const principals = Object.fromEntries([principal(), principal()]);
const addr = process.env.PAYDIRT_LISTEN ?? "127.0.0.1:19374";
const native = spawn(process.env.ROBOT_SERVER ?? resolve("../target/debug/robot-server"), {
  stdio: "inherit", env: { ...process.env, PAYDIRT_LISTEN: addr, PAYDIRT_PRINCIPALS: JSON.stringify(principals) },
});
const exited = new Promise(resolve => native.once("exit", resolve));
let server, browser;
try {
  let ready = false;
  for (let i = 0; i < 100 && native.exitCode === null; i++) {
    try { await fetch("http://" + addr); ready = true; break; }
    catch { await delay(50); }
  }
  assert(ready, "PostgreSQL server did not start");
  server = await createServer({ server: { host: "127.0.0.1", port: 0,
    proxy: { "/app.RobotService": "http://" + addr } }, logLevel: "error" });
  await server.listen();
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH });
  const page = await browser.newPage();
  page.on("pageerror", err => console.error(err));
  await page.goto(server.resolvedUrls.local[0]);
  await page.waitForFunction(() => typeof window.exercise === "function");
  for (const mode of ["postgres", "sqlite"]) {
    const id = await page.evaluate(async ({mode, principals}) => Promise.race([
      window.exercise(mode, principals),
      new Promise((_,reject) => setTimeout(() => reject(Error(mode + " suite timed out")), 30000)),
    ]), {mode, principals});
    console.log(mode, "passed Add/Get/List, atomic AddMany, tenant isolation, Watch snapshot, Patch, Erase and cancellation");
    if (mode === "sqlite") {
      await page.reload();
      await page.waitForFunction(() => typeof window.afterReload === "function");
      await page.evaluate(({principals,id}) => window.afterReload(principals,id), {principals,id});
      console.log("SQLite OPFS persisted after a page reload and two new Workers");
    }
  }
} finally {
  await browser?.close();
  await server?.close();
  native.kill();
  await exited;
}
