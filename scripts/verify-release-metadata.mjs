import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

const [packageJson, packageLock, extensionManifest, tauriConfig] =
  await Promise.all([
    readJson("package.json"),
    readJson("package-lock.json"),
    readJson("extension/manifest.json"),
    readJson("src-tauri/tauri.conf.json"),
  ]);
const cargoMetadata = JSON.parse(
  execFileSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1", "--locked"],
    { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
  ),
);
const workspaceIds = new Set(cargoMetadata.workspace_members ?? []);
const workspacePackages = (cargoMetadata.packages ?? []).filter((entry) =>
  workspaceIds.has(entry.id),
);

const expectedVersion = packageJson.version;
const versions = new Map([
  ["package.json", expectedVersion],
  ["package-lock.json", packageLock.version],
  ["package-lock.json packages root", packageLock.packages?.[""]?.version],
  ["extension/manifest.json", extensionManifest.version],
  ["src-tauri/tauri.conf.json", tauriConfig.version],
  ...workspacePackages.map((entry) => [
    `Cargo workspace package ${entry.name}`,
    entry.version,
  ]),
]);
const versionFailures = [...versions].filter(
  ([, version]) => version !== expectedVersion,
);
if (
  typeof expectedVersion !== "string" ||
  !/^\d+\.\d+\.\d+$/.test(expectedVersion) ||
  workspacePackages.length !== 3 ||
  versionFailures.length > 0
) {
  throw new Error(
    `release versions must be identical semantic versions: ${[...versions]
      .map(([file, version]) => `${file}=${String(version)}`)
      .join(", ")}`,
  );
}

const extensionId = deriveExtensionId(extensionManifest.key);
const expectedOrigin = `chrome-extension://${extensionId}`;
for (const file of [
  "crates/native-host/src/lib.rs",
  "src-tauri/src/native_host/install.rs",
]) {
  const source = await readFile(file, "utf8");
  const declarations = [
    ...source.matchAll(/pub const ALLOWED_ORIGIN: &str = "([^"]+)";/g),
  ];
  if (declarations.length !== 1) {
    throw new Error(`${file} must declare exactly one ALLOWED_ORIGIN`);
  }
  const origin = declarations[0][1];
  if (!/^chrome-extension:\/\/[a-p]{32}\/?$/.test(origin)) {
    throw new Error(`${file} ALLOWED_ORIGIN is malformed`);
  }
  if (origin.replace(/\/$/, "") !== expectedOrigin) {
    throw new Error(`${file} ALLOWED_ORIGIN does not match the manifest key`);
  }
}

const publishedId = process.env.CALLBACK_EXTENSION_ID?.trim();
if (publishedId) {
  if (!/^[a-p]{32}$/.test(publishedId)) {
    throw new Error("CALLBACK_EXTENSION_ID must be a 32-character Chrome ID");
  }
  if (publishedId !== extensionId) {
    throw new Error(
      "CALLBACK_EXTENSION_ID does not match the checked-in manifest key",
    );
  }
}

console.log(
  `Verified ${workspacePackages.length} Cargo packages at release version ${expectedVersion} and extension ID ${extensionId}${publishedId ? " against CALLBACK_EXTENSION_ID" : " (no published Store ID asserted)"}`,
);

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

function deriveExtensionId(key) {
  if (
    typeof key !== "string" ||
    !/^[A-Za-z0-9+/]+={0,2}$/.test(key) ||
    key.length % 4 !== 0
  ) {
    throw new Error("extension manifest key is not canonical base64");
  }
  const bytes = Buffer.from(key, "base64");
  if (bytes.length === 0) {
    throw new Error("extension manifest key is empty");
  }
  return [...createHash("sha256").update(bytes).digest().subarray(0, 16)]
    .flatMap((byte) => [byte >> 4, byte & 0x0f])
    .map((nibble) => String.fromCharCode("a".charCodeAt(0) + nibble))
    .join("");
}
