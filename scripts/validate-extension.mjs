import { access, readFile } from "node:fs/promises";
import { resolve } from "node:path";

const manifestPath = resolve(process.argv[2] ?? "extension/manifest.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
const failures = [];

if (manifest.manifest_version !== 3)
  failures.push("manifest_version must be 3");
if (!/^\d+(?:\.\d+){0,3}$/.test(manifest.version ?? "")) {
  failures.push("version must be a Chrome-compatible numeric version");
}
if (typeof manifest.key !== "string" || manifest.key.length === 0) {
  failures.push("key must pin the development extension ID");
}

const expectedPermissions = ["nativeMessaging", "storage"];
if (
  JSON.stringify([...(manifest.permissions ?? [])].sort()) !==
  JSON.stringify(expectedPermissions.sort())
) {
  failures.push("permissions must remain nativeMessaging and storage only");
}
if (manifest.permissions?.includes("storage.sync")) {
  failures.push("storage.sync is forbidden by the local-only contract");
}

const expectedHosts = ["https://app.slack.com/*", "https://mail.google.com/*"];
if (
  JSON.stringify([...(manifest.host_permissions ?? [])].sort()) !==
  JSON.stringify(expectedHosts)
) {
  failures.push("host permissions must remain Gmail web and Slack web only");
}

const referencedScripts = [
  manifest.background?.service_worker,
  ...(manifest.content_scripts ?? []).flatMap((entry) => entry.js ?? []),
].filter(Boolean);

if (manifestPath.includes(`${resolve("extension/dist")}`)) {
  const manifestDirectory = resolve(manifestPath, "..");
  await Promise.all(
    referencedScripts.map(async (script) => {
      try {
        await access(resolve(manifestDirectory, script));
      } catch {
        failures.push(`built manifest references missing ${script}`);
      }
    }),
  );
}

if (failures.length > 0) {
  throw new Error(failures.join("\n"));
}

console.log(`Validated ${manifestPath}`);
