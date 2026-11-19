#!/usr/bin/env node
// The schema watch's diff: what a newer Codex release changes in the part of the app-server
// protocol Hangar uses. How and when it runs: README.md ("Schema watch").
//
//   node scripts/codex-protocol/diff-schema.mjs <pinned> <latest> [--subset <subset.json>]
//
// <pinned> and <latest> are folders `codex app-server generate-json-schema --out` wrote, each with
// the `codex-version.txt` fetch-schema.sh and the workflow add (what `codex --version` printed).
// <pinned> must come from the release subset.json pins. Nothing here runs Codex.
//
// Both schemas go through generate.mjs's own reduction (the methods of subset.json and every type
// they reach), so what is compared is what Hangar's types are generated from, and nothing else:
// methods the subset names that are gone, and per method and type the fields added, removed, now
// required or no longer, whose type or enum values or alternatives changed. Descriptions and other
// documentation are not compared. Methods and types outside the subset are listed for information
// (methods only) and never change the exit code.
//
// Exit codes: 0 the subset is unchanged, 1 it changed, 2 the diff could not be made (a folder
// without schema, a pinned schema that is not subset.json's release).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  ProtocolError,
  flattenBundle,
  isObj,
  methodTables,
  pinnedSchema,
  readJson,
  reduceSchema,
  refName,
  schemaVersion,
} from "./generate.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const KINDS = ["clientRequests", "clientNotifications", "serverRequests", "serverNotifications"];

// ------------------------------------------------------------------------------------ one type

/**
 * What the diff compares of a schema node, keyed by its path inside the type: `` is the type
 * itself, `.name` a property, `[]` an array's items, `{}` a map's values, `<key>` one alternative
 * of a oneOf/anyOf (see variantKey).
 */
function describe(schema) {
  const nodes = new Map();
  walk(schema, "", null, nodes);
  return nodes;
}

function walk(schema, sub, parent, nodes) {
  let n = nodes.get(sub);
  if (!n) {
    n = { parent, field: false, required: false, closed: false, type: null, ref: null, values: new Set(), variants: new Set() };
    nodes.set(sub, n);
  }
  if (!isObj(schema)) return n;
  if (typeof schema.$ref === "string") n.ref = refName(schema.$ref);
  if (schema.type !== undefined) n.type = typeSignature(schema.type);
  if (schema.additionalProperties === false) n.closed = true;
  if ("const" in schema) n.values.add(JSON.stringify(schema.const));
  if (Array.isArray(schema.enum)) for (const v of schema.enum) n.values.add(JSON.stringify(v));
  const required = new Set(Array.isArray(schema.required) ? schema.required : []);
  for (const [name, prop] of Object.entries(isObj(schema.properties) ? schema.properties : {})) {
    const child = walk(prop, `${sub}.${name}`, sub, nodes);
    child.field = true;
    child.required = required.has(name);
  }
  if (isObj(schema.additionalProperties)) walk(schema.additionalProperties, `${sub}{}`, sub, nodes);
  if (isObj(schema.items)) walk(schema.items, `${sub}[]`, sub, nodes);
  if (Array.isArray(schema.allOf)) for (const part of schema.allOf) walk(part, sub, parent, nodes);
  for (const keyword of ["oneOf", "anyOf"]) {
    if (Array.isArray(schema[keyword])) alternatives(schema[keyword], sub, n, nodes);
  }
  return n;
}

const typeSignature = (type) => [].concat(type).sort().join(" | ");

/** The alternatives of a oneOf/anyOf: string alternatives are enum values, the others are walked. */
function alternatives(alts, sub, n, nodes) {
  const used = new Set();
  alts.forEach((alt, i) => {
    if (!isObj(alt)) return;
    if ((Array.isArray(alt.enum) || "const" in alt) && !alt.properties && !alt.$ref) {
      for (const v of "const" in alt ? [alt.const] : alt.enum) n.values.add(JSON.stringify(v));
      return;
    }
    let key = variantKey(alt) ?? `#${i + 1}`;
    while (used.has(key)) key += "'";
    used.add(key);
    n.variants.add(key);
    walk(alt, `${sub}<${key}>`, sub, nodes);
  });
}

/**
 * A name for one alternative that does not move when a field is added to it: its `$ref`, `null`,
 * the property that has one value (`type=agentMessage`), its only property (`{granular}`), its title.
 */
function variantKey(alt) {
  if (typeof alt.$ref === "string") return `$ref ${refName(alt.$ref)}`;
  if (alt.type === "null") return "null";
  const props = isObj(alt.properties) ? alt.properties : {};
  const names = Object.keys(props).sort();
  const single = (name) => {
    const p = props[name];
    if (!isObj(p)) return undefined;
    if ("const" in p) return p.const;
    return Array.isArray(p.enum) && p.enum.length === 1 ? p.enum[0] : undefined;
  };
  const tag = ["type", "method", "kind"].find((name) => single(name) !== undefined) ?? names.find((name) => single(name) !== undefined);
  if (tag !== undefined) return `${tag}=${single(tag)}`;
  if (names.length === 1) return `{${names[0]}}`;
  if (typeof alt.title === "string") return alt.title;
  if (names.length) return `{${names.join(",")}}`;
  if (alt.type !== undefined) return `type ${typeSignature(alt.type)}`;
  return null;
}

/** A node in a few words: "string | null", "AskForApproval", "enum("a", "b")". */
function shape(n) {
  const parts = [];
  if (n.ref) parts.push(n.ref);
  else if (n.type) parts.push(n.type);
  if (n.variants.size) parts.push([...n.variants].map((v) => v.replace(/^\$ref /, "")).join(" | "));
  if (n.values.size) parts.push(`enum(${[...n.values].join(", ")})`);
  return parts.join(" ") || "any";
}

const setDiff = (a, b) => [...a].filter((x) => !b.has(x));

/** Pushes the changes between two descriptions of the same thing (`rootLabel` + path is shown). */
function diffNodes(scope, rootLabel, a, b, changes) {
  const add = (kind, sub, detail = "") => changes.push({ scope, path: `${rootLabel}${sub}`, kind, detail });
  for (const sub of [...new Set([...a.keys(), ...b.keys()])].sort()) {
    const x = a.get(sub);
    const y = b.get(sub);
    if (!x || !y) {
      // Only a field that appears or goes inside something both schemas have: what is inside an
      // alternative or a field that is new (or gone) is that alternative's or field's own news.
      const n = x ?? y;
      if (!n.field || !(x ? b : a).has(n.parent)) continue;
      if (x) add("field-removed", sub);
      else add("field-added", sub, `${shape(n)}${n.required ? ", required" : ""}`);
      continue;
    }
    if (x.field && y.field && x.required !== y.required) add(y.required ? "field-required" : "field-optional", sub);
    if (x.type !== y.type) add("type-changed", sub, `${x.type ?? "none"} -> ${y.type ?? "none"}`);
    if (x.ref !== y.ref) add("ref-changed", sub, `${x.ref ?? "none"} -> ${y.ref ?? "none"}`);
    if (x.closed !== y.closed) add(y.closed ? "closed" : "opened", sub);
    const valuesAdded = setDiff(y.values, x.values);
    const valuesRemoved = setDiff(x.values, y.values);
    if (valuesAdded.length) add("enum-added", sub, valuesAdded.join(", "));
    if (valuesRemoved.length) add("enum-removed", sub, valuesRemoved.join(", "));
    const variantsAdded = setDiff(y.variants, x.variants);
    const variantsRemoved = setDiff(x.variants, y.variants);
    if (variantsAdded.length) add("variant-added", sub, variantsAdded.join(", "));
    if (variantsRemoved.length) add("variant-removed", sub, variantsRemoved.join(", "));
  }
}

// ------------------------------------------------------------------------------------ the diff

/**
 * The changes of the subset between two flattened schemas (generate.mjs's flattenBundle):
 * `changes` count (they decide the exit code), `notes` are methods outside the subset, for information.
 */
export function diffSchemas({ pinned, latest, subset }) {
  // The pinned schema must fit subset.json whole; the comparison below leaves out what the latest lacks.
  try {
    reduceSchema(pinned, subset);
  } catch (e) {
    if (!(e instanceof ProtocolError)) throw e;
    throw new ProtocolError(`the pinned schema does not fit subset.json: ${e.message}`);
  }
  const changes = [];
  const pinnedTables = methodTables(pinned);
  const latestTables = methodTables(latest);
  const kept = { ...subset };
  for (const kind of KINDS) {
    kept[kind] = subset[kind].filter((method) => method in latestTables[kind]);
    for (const method of subset[kind]) {
      if (!(method in latestTables[kind])) changes.push({ scope: `${kind} ${method}`, path: "", kind: "method-removed", detail: "" });
    }
  }

  let reducedLatest;
  try {
    reducedLatest = reduceSchema(latest, kept);
  } catch (e) {
    if (!(e instanceof ProtocolError)) throw e;
    changes.push({ scope: "latest schema", path: "", kind: "unreducible", detail: e.message });
  }
  if (reducedLatest) diffReduced(reduceSchema(pinned, kept), reducedLatest, kept, changes);

  const notes = [];
  for (const kind of KINDS) {
    const before = new Set(Object.keys(pinnedTables[kind]));
    const after = new Set(Object.keys(latestTables[kind]));
    const inSubset = new Set(subset[kind]);
    const added = [...after].filter((m) => !before.has(m));
    const removed = [...before].filter((m) => !after.has(m) && !inSubset.has(m));
    if (added.length || removed.length) notes.push({ kind, added, removed });
  }
  return { changes, notes };
}

function diffReduced(a, b, kept, changes) {
  for (const kind of KINDS) {
    for (const method of kept[kind]) {
      const x = a[kind][method];
      const y = b[kind][method];
      const scope = `${kind} ${method}`;
      if (x.paramsRequired !== y.paramsRequired) {
        changes.push({ scope, path: "params", kind: y.paramsRequired ? "field-required" : "field-optional", detail: "" });
      }
      if (x.response !== y.response) {
        changes.push({ scope, path: "response", kind: "ref-changed", detail: `${x.response ?? "none"} -> ${y.response ?? "none"}` });
      }
      diffNodes(scope, "params", describe(x.params ?? {}), describe(y.params ?? {}), changes);
    }
  }
  const names = new Set([...Object.keys(a.definitions), ...Object.keys(b.definitions)]);
  for (const name of [...names].sort()) {
    const scope = `type ${name}`;
    if (!(name in b.definitions)) changes.push({ scope, path: "", kind: "type-removed", detail: "" });
    else if (!(name in a.definitions)) changes.push({ scope, path: "", kind: "type-added", detail: "" });
    else diffNodes(scope, "", describe(a.definitions[name]), describe(b.definitions[name]), changes);
  }
}

// ------------------------------------------------------------------------------------ the text

// No backtick in what is printed: the workflow puts it in a code fence.
const KIND_TEXT = {
  "method-removed": ["-", "method removed"],
  "field-added": ["+", "field added"],
  "field-removed": ["-", "field removed"],
  "field-required": ["~", "now required"],
  "field-optional": ["~", "no longer required"],
  "type-changed": ["~", "type changed"],
  "ref-changed": ["~", "refers to another type"],
  closed: ["~", "now closed (additionalProperties: false)"],
  opened: ["~", "no longer closed"],
  "enum-added": ["+", "enum values added"],
  "enum-removed": ["-", "enum values removed"],
  "variant-added": ["+", "alternatives added"],
  "variant-removed": ["-", "alternatives removed"],
  "type-added": ["+", "type added (reachable from the subset now)"],
  "type-removed": ["-", "type removed (no longer reachable from the subset)"],
  unreducible: ["!", "cannot be reduced to the subset"],
};

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** The readable diff: `labels` are what `codex --version` printed for each side. */
export function render({ changes, notes }, labels) {
  const lines = [
    "Codex app-server schema, restricted to the subset Hangar uses (scripts/codex-protocol/subset.json)",
    `  pinned  ${labels.pinned}`,
    `  latest  ${labels.latest}`,
  ];
  let scope;
  for (const c of changes) {
    if (c.scope !== scope) lines.push("", c.scope);
    scope = c.scope;
    const [glyph, text] = KIND_TEXT[c.kind];
    lines.push(`  ${glyph} ${c.path ? `${c.path}  ` : ""}${text}${c.detail ? `: ${c.detail}` : ""}`);
  }
  lines.push("");
  lines.push(changes.length ? `${plural(changes.length, "change")} in the subset.` : "The subset is unchanged.");
  if (notes.length) {
    lines.push("", "Outside the subset (for information, never a change of the subset):");
    for (const { kind, added, removed } of notes) {
      const list = (names) => (names.length > 30 ? `${names.slice(0, 30).join(", ")}, … (${names.length} in all)` : names.join(", "));
      if (added.length) lines.push(`  ${kind} added: ${list(added)}`);
      if (removed.length) lines.push(`  ${kind} removed: ${list(removed)}`);
    }
  }
  return `${lines.join("\n")}\n`;
}

// ------------------------------------------------------------------------------------ main

function usage() {
  console.error("usage: node scripts/codex-protocol/diff-schema.mjs <pinned schema folder> <latest schema folder> [--subset <subset.json>]");
  process.exit(2);
}

function main(argv) {
  const folders = [];
  let subsetFile = path.join(HERE, "subset.json");
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--subset") subsetFile = argv[++i] ?? usage();
    else if (argv[i].startsWith("-")) usage();
    else folders.push(argv[i]);
  }
  if (folders.length !== 2) usage();
  const [pinnedDir, latestDir] = folders.map((f) => path.resolve(f));
  const subset = readJson(subsetFile);

  const pinned = pinnedSchema(pinnedDir, subset);
  const labels = { pinned: schemaVersion(pinnedDir), latest: schemaVersion(latestDir) };
  // A latest schema that cannot be read at all is a change of the protocol, not a failed diff.
  let latest;
  let unreadable;
  try {
    latest = flattenBundle(latestDir);
  } catch (e) {
    if (!(e instanceof ProtocolError)) throw e;
    unreadable = e.message;
  }
  const result = latest
    ? diffSchemas({ pinned, latest, subset })
    : { changes: [{ scope: "latest schema", path: "", kind: "unreducible", detail: unreadable }], notes: [] };
  process.stdout.write(render(result, labels));
  process.exit(result.changes.length ? 1 : 0);
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url))) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(`diff-schema: ${e.message}`);
    process.exit(2);
  }
}
