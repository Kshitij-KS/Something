import { access, readFile } from "node:fs/promises";
import { dirname, relative, resolve, sep } from "node:path";

const sourceManifestPath = resolve("extension/manifest.json");
const builtManifestPath = resolve("extension/dist/manifest.json");
const manifestPath = resolve(process.argv[2] ?? sourceManifestPath);
const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
const failures = [];

if (manifest.manifest_version !== 3) {
  failures.push("manifest_version must be 3");
}
if (!/^\d+(?:\.\d+){0,3}$/.test(manifest.version ?? "")) {
  failures.push("version must be a Chrome-compatible numeric version");
}
if (typeof manifest.key !== "string" || manifest.key.length === 0) {
  failures.push("key must pin the development extension ID");
}

const expectedPermissions = ["nativeMessaging", "storage"];
if (
  JSON.stringify([...(manifest.permissions ?? [])].sort()) !==
  JSON.stringify([...expectedPermissions].sort())
) {
  failures.push("permissions must remain nativeMessaging and storage only");
}

const expectedHosts = ["https://app.slack.com/*", "https://mail.google.com/*"];
if (
  JSON.stringify([...(manifest.host_permissions ?? [])].sort()) !==
  JSON.stringify([...expectedHosts].sort())
) {
  failures.push("host permissions must remain Gmail web and Slack web only");
}

const referencedScripts = [
  manifest.background?.service_worker,
  ...(manifest.content_scripts ?? []).flatMap((entry) => entry.js ?? []),
].filter(Boolean);
if (referencedScripts.length === 0) {
  failures.push("manifest must reference extension scripts");
}

if (manifestPath === builtManifestPath) {
  const sourceManifest = JSON.parse(await readFile(sourceManifestPath, "utf8"));
  if (JSON.stringify(manifest) !== JSON.stringify(sourceManifest)) {
    failures.push("built manifest must exactly match extension/manifest.json");
  }
  const manifestDirectory = dirname(manifestPath);
  const requiredFiles = [...referencedScripts, "selectors.json"];
  await Promise.all(
    requiredFiles.map(async (script) => {
      const scriptPath = resolve(manifestDirectory, script);
      if (!scriptPath.startsWith(`${manifestDirectory}${sep}`)) {
        failures.push(`built manifest contains unsafe path ${script}`);
        return;
      }
      try {
        await access(scriptPath);
      } catch {
        failures.push(`built extension is missing ${script}`);
      }
    }),
  );
}

if (failures.length > 0) {
  throw new Error(failures.join("\n"));
}

console.log(`Validated ${relative(process.cwd(), manifestPath)}`);
