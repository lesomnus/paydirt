const base = process.env.PAYDIRT_URL ?? "http://127.0.0.1:8080";
const response = await fetch(new URL("/app.SettingService/Get", base), {
  method: "POST",
  headers: { "Content-Type": "application/json", "Connect-Protocol-Version": "1" },
  body: JSON.stringify({ ref: { alias: process.argv[2] ?? "service" } }),
});
const message = await response.json();
if (!response.ok) throw Error(message.code + ": " + message.message);
console.log(message.value);
