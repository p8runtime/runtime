#!/usr/bin/env bun
import { spawn } from "node:child_process";
import { readdir, readFile, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(process.cwd());
let suiteArg = "tests";
let skipArgs = [];

const rawArgs = process.argv.slice(2);
let suiteSpecified = false;
for (let i = 0; i < rawArgs.length; i += 1) {
  const arg = rawArgs[i];
  if (arg === "--skip") {
    if (i + 1 >= rawArgs.length) {
      throw new Error("--skip requires a value");
    }
    skipArgs.push(rawArgs[++i]);
  } else if (arg.startsWith("--skip=")) {
    skipArgs.push(arg.slice("--skip=".length));
  } else if (!suiteSpecified) {
    suiteArg = arg;
    suiteSpecified = true;
  } else {
    throw new Error(`Unexpected argument: ${arg}`);
  }
}

const suiteDir = path.resolve(repoRoot, suiteArg);

const phpBinary = process.env.PHP_BIN || "php";
const gildBinaryCandidate = process.env.GILD_NATIVE_BIN
  ? path.resolve(process.env.GILD_NATIVE_BIN)
  : path.resolve(repoRoot, "target/release/php");

const skipPatterns = [];
for (const raw of skipArgs.flatMap((arg) => arg.split(","))) {
  const trimmed = raw.trim();
  if (!trimmed) {
    continue;
  }
  skipPatterns.push(path.resolve(repoRoot, trimmed));
}

async function collectPhpFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  let files = [];
  for (const entry of entries) {
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name.startsWith("_")) {
        continue;
      }
      files = files.concat(await collectPhpFiles(entryPath));
    } else if (entry.isFile() && entry.name.endsWith(".php")) {
      files.push(entryPath);
    }
  }
  return files.sort();
}

async function runBinary(binary, scriptPath) {
  return new Promise((resolve, reject) => {
    const proc = spawn(binary, [scriptPath], {
      env: { ...process.env },
      cwd: path.dirname(scriptPath),
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    proc.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });

    proc.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });

    proc.on("error", reject);
    proc.on("close", (code) => {
      resolve({ stdout, stderr, code: code ?? 0 });
    });
  });
}

function indent(text) {
  if (text === "") {
    return "    (empty)";
  }
  return text
    .split(/\r?\n/)
    .map((line) => `    ${line}`)
    .join("\n");
}

const COLOR = {
  reset: "\x1b[0m",
  green: "\x1b[32m",
  red: "\x1b[31m",
  yellow: "\x1b[33m",
};

async function main() {
  console.log(`Running suite in ${suiteDir}`);
  const exists = await stat(suiteDir).then(() => true).catch(() => false);
  if (!exists) {
    console.error(`${COLOR.red}Suite directory not found: ${suiteDir}${COLOR.reset}`);
    process.exit(1);
  }

  const files = (await collectPhpFiles(suiteDir)).filter((filePath) => !shouldSkip(filePath));
  const pendingFiles = await collectPendingFiles(suiteDir);
  if (files.length === 0) {
    printPendingFiles(pendingFiles);
    console.log("No PHP files found.");
    const total = pendingFiles.length;
    const percent = total === 0 ? 0 : 0;
    console.log(
      `\n${COLOR.green}Summary: 0/${total} passed. ${percent}% overall. ${pendingFiles.length} tests pending language implementation.${COLOR.reset}`
    );
    console.log("");
    return;
  }

  const gildBinary = await resolveGildBinary();
  let failures = 0;

  for (const scriptPath of files) {
    const relative = path.relative(repoRoot, scriptPath);
    process.stdout.write(`${relative} ... `);

    const directives = await loadDirectives(scriptPath);
    const officialRes = await runBinary(phpBinary, scriptPath);
    const gildRes = await runBinary(gildBinary, scriptPath);
    const official = sanitizeResult(officialRes);
    const gild = sanitizeResult(gildRes);

    const matches =
      official.stdout === gild.stdout &&
      official.stderr === gild.stderr &&
      official.code === gild.code;

    if (matches) {
      console.log(`${COLOR.green}ok${COLOR.reset}`);
      continue;
    }

    if (directives.shapes) {
      const officialShapeOk = matchesShapes(directives.shapes, official);
      const gildShapeOk = matchesShapes(directives.shapes, gild);
      const stderrMatch = directives.shapes.stderr
        ? true
        : official.stderr === gild.stderr;
      if (officialShapeOk && gildShapeOk && official.code === gild.code && stderrMatch) {
        console.log(`${COLOR.yellow}shape ok${COLOR.reset}`);
        continue;
      }
    }

    if (
      directives.nondeterministic &&
      official.code === gild.code &&
      (gild.stdout !== "" || gild.stderr !== "")
    ) {
      console.log(`${COLOR.yellow}soft ok${COLOR.reset}`);
      continue;
    }

    failures += 1;
    console.log(`${COLOR.red}FAILED${COLOR.reset}`);
    if (official.code !== gild.code) {
      console.log(`  exit codes differ: official=${official.code} gild=${gild.code}`);
    }

    printBlock("stdout", official.stdout, gild.stdout);
    printBlock("stderr", official.stderr, gild.stderr);
  }

  const matched = files.length - failures;
  const total = files.length + pendingFiles.length;
  const percent = total === 0 ? 0 : Math.round((matched / total) * 100);
  const summaryColor = failures > 0 ? COLOR.red : COLOR.green;
  printPendingFiles(pendingFiles);
  console.log(
    `\n${summaryColor}Summary: ${matched}/${total} passed. ${percent}% overall. ${pendingFiles.length} tests pending language implementation.${COLOR.reset}`
  );
  console.log("");
  if (failures > 0) {
    process.exit(1);
  }
}

async function loadDirectives(scriptPath) {
  try {
    const content = await readFile(scriptPath, "utf8");
    return {
      nondeterministic: content.includes("@nondeterministic"),
      shapes: parseShapeDirectives(content),
    };
  } catch {
    return { nondeterministic: false, shapes: null };
  }
}

function parseShapeDirectives(content) {
  const shapes = {};
  for (const line of content.split(/\r?\n/)) {
    const match = line.match(/@shape\s+(\w+)\s*=\s*([^\s]+)/);
    if (!match) {
      continue;
    }
    const target = match[1];
    const shape = match[2];
    shapes[target] = shape;
  }
  return Object.keys(shapes).length > 0 ? shapes : null;
}

function matchesShapes(shapes, result) {
  for (const [target, shape] of Object.entries(shapes)) {
    const output = target === "stderr" ? result.stderr : result.stdout;
    if (!matchesShape(shape, output)) {
      return false;
    }
  }
  return true;
}

function matchesShape(shape, output) {
  const trimmed = output.trim();
  if (shape === "int") {
    return /^-?\d+$/.test(trimmed);
  }
  if (shape === "float") {
    return /^-?\d+\.\d+(?:[eE][+-]?\d+)?$/.test(trimmed);
  }
  if (shape === "number") {
    const value = Number(trimmed);
    return Number.isFinite(value);
  }
  if (shape === "string") {
    return trimmed.length > 0;
  }
  if (shape.startsWith("array<") && shape.endsWith(">")) {
    const inner = shape.slice("array<".length, -1);
    const values = parsePrintRArray(output);
    if (values === null) {
      return false;
    }
    return values.every((val) => matchesShape(inner, val));
  }
  if (shape.startsWith("lines<") && shape.endsWith(">")) {
    const inner = shape.slice("lines<".length, -1);
    const lines = output
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
    if (lines.length === 0) {
      return false;
    }
    return lines.every((line) => matchesShape(inner, line));
  }
  return false;
}

function parsePrintRArray(output) {
  const match = output.match(/Array\s*\(([\s\S]*)\)/);
  if (!match) {
    return null;
  }
  const body = match[1];
  const values = [];
  for (const line of body.split(/\r?\n/)) {
    const entry = line.match(/\[\s*[^\]]+\s*\]\s*=>\s*(.*)/);
    if (!entry) {
      continue;
    }
    values.push(entry[1].trim());
  }
  return values;
}

async function resolveGildBinary() {
  if (await exists(gildBinaryCandidate)) {
    return gildBinaryCandidate;
  }

  const fallback = path.resolve(repoRoot, "target/debug/php");
  if (await exists(fallback)) {
    return fallback;
  }

  throw new Error(
    `gild native PHP binary not found; build it with 'cargo build --bin php' or set GILD_NATIVE_BIN`
  );
}

async function exists(p) {
  try {
    await stat(p);
    return true;
  } catch {
    return false;
  }
}

function printBlock(label, official, gild) {
  if (official === gild) {
    return;
  }

  console.log(`  ${label}:`);
  console.log("    official:");
  console.log(indent(official));
  console.log("    gild:");
  console.log(indent(gild));
}

function sanitizeResult({ stdout, stderr, code }) {
  return {
    stdout: sanitizeStream(stdout),
    stderr: sanitizeStream(stderr),
    code,
  };
}

function sanitizeStream(text) {
  const sanitized = text
    .split(/\r?\n/)
    .filter((line) => !line.startsWith("[PthreadsExtension]"))
    .join("\n")
    .replace(/\n{3,}/g, "\n\n");
  const trimmedStart = sanitized.replace(/^\s*\n/, "");
  return trimmedStart
    .trimEnd()
    .concat(text.endsWith("\n") ? "\n" : "");
}

function shouldSkip(filePath) {
  if (skipPatterns.length === 0) {
    return false;
  }
  const resolved = path.resolve(filePath);
  return skipPatterns.some((pattern) => resolved === pattern || resolved.startsWith(pattern + path.sep));
}

async function collectPhpFilesInDir(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  let files = [];
  for (const entry of entries) {
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files = files.concat(await collectPhpFilesInDir(entryPath));
    } else if (entry.isFile() && entry.name.endsWith(".php")) {
      files.push(entryPath);
    }
  }
  return files.sort();
}

async function collectPendingFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  let pending = [];
  for (const entry of entries) {
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name.startsWith("_")) {
        pending = pending.concat(await collectPhpFilesInDir(entryPath));
        continue;
      }
      pending = pending.concat(await collectPendingFiles(entryPath));
    }
  }
  return pending.sort();
}

function printPendingFiles(pendingFiles) {
  if (pendingFiles.length === 0) {
    return;
  }
  for (const pending of pendingFiles) {
    printPendingLine(pending);
  }
}

function printPendingLine(pending) {
  const relative = path.relative(repoRoot, pending);
  console.log(`${relative} ... ${COLOR.yellow}pending${COLOR.reset}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
