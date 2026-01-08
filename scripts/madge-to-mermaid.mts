import * as fs from "node:fs";

type MadgeJson = Record<string, string[]>;

const inputPath = process.argv[2];
const outputPath = process.argv[3];
const direction = process.argv[4] ?? "LR";

if (!inputPath || !outputPath) {
  console.error("Usage: node madge-to-mermaid.mts <input.json> <output.mmd> [LR|TB|RL|BT]");
  process.exit(2);
}

const raw = fs.readFileSync(inputPath, "utf8");
const data: MadgeJson = JSON.parse(raw);

const idByPath = new Map<string, string>();
let nextId = 0;

function getId(p: string): string {
  const existing = idByPath.get(p);
  if (existing) return existing;
  const id = `n${++nextId}`;
  idByPath.set(p, id);
  return id;
}

function escLabel(s: string): string {
  return String(s).replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

const lines: string[] = [];
lines.push(`flowchart ${direction}`);

const emittedNode = new Set<string>();
const emittedEdge = new Set<string>();

function emitNode(p: string): void {
  const id = getId(p);
  if (emittedNode.has(id)) return;
  emittedNode.add(id);
  lines.push(`  ${id}["${escLabel(p)}"]`);
}

function emitEdge(from: string, to: string): void {
  const a = getId(from);
  const b = getId(to);
  const k = `${a}->${b}`;
  if (emittedEdge.has(k)) return;
  emittedEdge.add(k);
  lines.push(`  ${a} --> ${b}`);
}

for (const [from, deps] of Object.entries(data)) {
  emitNode(from);
  for (const to of deps) {
    emitNode(to);
    emitEdge(from, to);
  }
}

fs.writeFileSync(outputPath, lines.join("\n") + "\n", "utf8");
