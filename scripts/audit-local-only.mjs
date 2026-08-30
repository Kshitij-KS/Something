import { readFile, readdir } from "node:fs/promises";
import { extname, join, relative } from "node:path";

const sourceRoots = [
  "src",
  "src-tauri/src",
  "crates/native-host/src",
  "crates/protocol/src",
  "extension/src",
];
const sourceExtensions = new Set([".rs", ".ts", ".tsx", ".js", ".mjs"]);
const networkRules = [
  ["Rust network module", /\b(?:std|tokio|mio)::net\b/g],
  ["TCP or UDP API", /\b(?:TcpListener|TcpStream|UdpSocket|UdpFramed)\b/g],
  [
    "Windows Winsock API",
    /\b(?:Win32_Networking_WinSock|WSAStartup|WSASocket)\b/g,
  ],
  [
    "Node network module",
    /(?:node:|require\s*\(\s*["'])(?:net|dgram|http|https)\b/g,
  ],
  ["browser fetch", /\bfetch\s*\(/g],
  ["XMLHttpRequest", /\bXMLHttpRequest\b/g],
  ["WebSocket", /\bWebSocket\b/g],
  ["EventSource", /\bEventSource\b/g],
  ["sendBeacon", /\bsendBeacon\b/g],
  ["Node server", /\bcreateServer\s*\(/g],
  ["remote dynamic import", /\bimport\s*\(\s*["'`]https?:\/\//g],
];
const extensionRules = [
  ["synchronized browser storage", /\b(?:chrome|browser)\.storage\.sync\b/g],
  [
    "runtime selector-pack lookup",
    /\b(?:runtime\.getURL|fetch|XMLHttpRequest)[\s\S]{0,160}selectors\.json\b/g,
  ],
];
const rawLogRule =
  /(?:console\.(?:log|debug|info|warn|error)\s*\(|(?:tracing::(?:trace|debug|info|warn|error)|println|eprintln)!\s*\()[^;]{0,500}\b(?:rawMessage|raw_message)\b/gs;
const telemetryName =
  /(?:^|[/@_-])(?:sentry|opentelemetry|datadog|newrelic|new-relic|amplitude|mixpanel|segment|posthog|telemetrydeck|bugsnag|rollbar)(?:$|[/@_-])/i;

const failures = [];
const sourceFiles = (
  await Promise.all(
    sourceRoots.map((root) => listFiles(root, sourceExtensions)),
  )
).flat();
for (const path of sourceFiles) {
  const text = await readFile(path, "utf8");
  checkRules(path, text, networkRules);
  if (path.startsWith("extension/")) {
    checkRules(path, text, extensionRules);
  }
  for (const match of text.matchAll(rawLogRule)) {
    failures.push(failure(path, text, match.index, "raw message in log call"));
  }
}

const builtFiles = await listFiles("extension/dist", new Set([".js"]));
for (const required of [
  "extension/dist/background.js",
  "extension/dist/content.js",
]) {
  if (!builtFiles.includes(required)) {
    failures.push(`${required}: missing built extension entry`);
  }
}
for (const path of builtFiles) {
  const text = await readFile(path, "utf8");
  checkRules(path, text, [...networkRules, ...extensionRules]);
}

await auditDependencies();

if (failures.length > 0) {
  throw new Error(`Local-only static audit failed:\n${failures.join("\n")}`);
}
console.log(
  `Local-only static audit passed (${sourceFiles.length} first-party source files, ${builtFiles.length} built extension files).`,
);
console.log(
  "Installed-process TCP/UDP inspection remains a separate manual Windows check.",
);

function checkRules(path, text, rules) {
  for (const [label, pattern] of rules) {
    pattern.lastIndex = 0;
    for (const match of text.matchAll(pattern)) {
      failures.push(failure(path, text, match.index, label));
    }
  }
}

function failure(path, text, index, label) {
  const line = text.slice(0, index ?? 0).split(/\r?\n/).length;
  return `${path}:${line}: ${label}`;
}

async function listFiles(root, extensions) {
  let entries;
  try {
    entries = await readdir(root, { withFileTypes: true });
  } catch {
    return [];
  }
  const files = [];
  for (const entry of entries) {
    const path = join(root, entry.name).replaceAll("\\", "/");
    if (entry.isDirectory()) {
      files.push(...(await listFiles(path, extensions)));
    } else if (entry.isFile() && extensions.has(extname(entry.name))) {
      files.push(path);
    }
  }
  return files.sort();
}

async function auditDependencies() {
  const packageJson = JSON.parse(await readFile("package.json", "utf8"));
  const npmNames = new Set([
    ...Object.keys(packageJson.dependencies ?? {}),
    ...Object.keys(packageJson.devDependencies ?? {}),
  ]);
  const packageLock = JSON.parse(await readFile("package-lock.json", "utf8"));
  for (const [path, metadata] of Object.entries(packageLock.packages ?? {})) {
    if (metadata?.name) npmNames.add(metadata.name);
    const marker = "node_modules/";
    const markerIndex = path.lastIndexOf(marker);
    if (markerIndex >= 0) npmNames.add(path.slice(markerIndex + marker.length));
  }
  for (const name of npmNames) {
    if (telemetryName.test(name)) {
      failures.push(`package dependency ${name}: telemetry dependency`);
    }
  }

  const cargoFiles = [
    "Cargo.toml",
    ...(await listFiles("crates", new Set([".toml"]))),
    ...(await listFiles("src-tauri", new Set([".toml"]))),
    "Cargo.lock",
  ];
  for (const path of [...new Set(cargoFiles)]) {
    if (!path.endsWith("Cargo.toml") && !path.endsWith("Cargo.lock")) continue;
    const text = await readFile(path, "utf8");
    const names = [
      ...text.matchAll(
        /(?:^|\r?\n)(?:name\s*=\s*"([^"]+)"|([A-Za-z0-9_-]+)\s*=)/g,
      ),
    ]
      .flatMap((match) => [match[1], match[2]])
      .filter(Boolean);
    for (const name of names) {
      if (telemetryName.test(name)) {
        failures.push(`${relative(".", path)}: telemetry dependency ${name}`);
      }
    }
  }
}
