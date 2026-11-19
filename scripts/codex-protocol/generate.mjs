#!/usr/bin/env node
// Rust types for the Codex app-server protocol, generated from the JSON Schema the pinned Codex
// release prints (`codex app-server generate-json-schema`), reduced to the methods Hangar uses
// (subset.json), into crates/hangar-core/src/codex_app/generated.rs. How and when: README.md.
//
//   node scripts/codex-protocol/generate.mjs --schema <dir>   a folder `generate-json-schema --out`
//                                                             wrote (fetch-schema.sh brings one back
//                                                             from the build machine)
//   node scripts/codex-protocol/generate.mjs --codex <path>   runs that codex binary itself (never on
//                                                             the person's PC: Codex is not installed there)
//   node scripts/codex-protocol/generate.mjs                  the Rust again from schema.subset.json
//                                                             (a generator change, the same pin)
//   … --check                                                 writes nothing; exit 1 when a file differs
//
// One pinned Codex release, the schema as its own binary prints it, a few documented adjustments,
// the result checked in, regenerated only when the pin moves.
//
// Lenient by construction, so a newer Codex never breaks decoding: unknown fields are ignored
// (no `deny_unknown_fields`), every string enum (but an object's `type` marker) and every union
// ends with an `Unrecognized` variant that keeps what it did not know, and the method enums keep an
// unknown notification or request whole. A field the schema requires stays required.
//
// It also writes, from the schema alone, a JSON instance of every type and every variant
// (codex_app/testdata/schema-samples.jsonl) that the Rust tests read back.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..", "..");
const SUBSET = path.join(HERE, "subset.json");
const SCHEMA_SUBSET = path.join(HERE, "schema.subset.json");
const OUT = path.join(ROOT, "crates", "hangar-core", "src", "codex_app", "generated.rs");
const SAMPLES = path.join(ROOT, "crates", "hangar-core", "src", "codex_app", "testdata", "schema-samples.jsonl");
const BUNDLE = "codex_app_server_protocol.schemas.json";
const VERSION_FILE = "codex-version.txt";
const FALLBACK = "Unrecognized";

function fail(message) {
  console.error(`codex-protocol: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const args = { check: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--check") args.check = true;
    else if (a === "--schema") args.schema = argv[++i];
    else if (a === "--codex") args.codex = argv[++i];
    else if (a === "-h" || a === "--help") {
      console.log(fs.readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").slice(1, 13).join("\n"));
      process.exit(0);
    } else fail(`unknown argument ${a} (see --help)`);
  }
  if (args.schema && args.codex) fail("--schema and --codex exclude each other");
  return args;
}

const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

// ------------------------------------------------------------------------------------ the schema

/** Runs the pinned codex to write its schema bundle into a new temporary folder. */
function schemaFromCodex(codex, subset) {
  const version = execFileSync(codex, ["--version"], { encoding: "utf8" }).trim();
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-protocol-"));
  // An empty CODEX_HOME: the schema must not depend on whoever runs this.
  const home = path.join(dir, "home");
  fs.mkdirSync(home);
  const out = path.join(dir, "schema");
  const argv = ["app-server", "generate-json-schema", "--out", out];
  if (subset.experimental) argv.push("--experimental");
  execFileSync(codex, argv, { stdio: "inherit", env: { ...process.env, CODEX_HOME: home } });
  fs.writeFileSync(path.join(out, VERSION_FILE), `${version}\n`);
  return out;
}

/**
 * The bundle Codex writes keeps the v2 types under `definitions.v2`; Hangar's types live in one
 * namespace. A name in both must be the same type (RequestId and AbsolutePathBuf are, at 0.170.0).
 */
function flattenBundle(dir, subset) {
  const versionFile = path.join(dir, VERSION_FILE);
  if (!fs.existsSync(versionFile)) fail(`${versionFile} is missing: which codex wrote this schema?`);
  const version = fs.readFileSync(versionFile, "utf8").trim();
  if (version !== `codex-cli ${subset.codex}`)
    fail(`the schema in ${dir} comes from "${version}", subset.json pins codex-cli ${subset.codex}`);
  const bundle = readJson(path.join(dir, BUNDLE)).definitions;
  const v2 = bundle.v2 ?? {};
  const rewrite = (node) => {
    if (Array.isArray(node)) return node.map(rewrite);
    if (!isObj(node)) return node;
    const out = {};
    for (const [k, v] of Object.entries(node)) {
      if (k === "$ref" && typeof v === "string") out[k] = v.replace(/^#\/definitions\/v2\//, "#/definitions/");
      else out[k] = rewrite(v);
    }
    return out;
  };
  const clean = (schema) => {
    const { $schema: _s, ...rest } = rewrite(schema);
    return rest;
  };
  const defs = {};
  for (const [name, schema] of Object.entries(v2)) defs[name] = clean(schema);
  for (const [name, schema] of Object.entries(bundle)) {
    if (name === "v2") continue;
    const s = clean(schema);
    if (name in defs) {
      const same = (x) => JSON.stringify({ ...x, title: undefined });
      if (same(defs[name]) !== same(s))
        fail(`"${name}" is two different types in the bundle (root and v2): name one of them in the generator`);
      continue;
    }
    defs[name] = s;
  }
  return defs;
}

/** method → {params, paramsRequired} from one of the four method unions of the bundle. */
function methodTable(defs, unionName) {
  const table = {};
  for (const alt of defs[unionName]?.oneOf ?? []) {
    const method = alt.properties?.method?.enum?.[0];
    if (typeof method !== "string") fail(`${unionName}: an alternative without a method`);
    const params = alt.properties.params;
    table[method] = { params: params ?? null, paramsRequired: (alt.required ?? []).includes("params") };
  }
  return table;
}

function pascalMethod(method) {
  return method
    .split(/[/_\-.]/)
    .filter(Boolean)
    .map((s) => s[0].toUpperCase() + s.slice(1))
    .join("");
}

/** The response type of a request, by Codex's naming (`…Params` → `…Response`), or the subset's override. */
function responseType(method, entry, defs, overrides) {
  const candidates = [];
  if (overrides[method]) candidates.push(overrides[method]);
  else {
    const ref = refName(entry.params?.$ref ?? entry.params?.anyOf?.find((a) => a.$ref)?.$ref);
    if (ref?.endsWith("Params")) candidates.push(ref.replace(/Params$/, "Response"));
    candidates.push(`${pascalMethod(method)}Response`);
  }
  const found = candidates.find((c) => c in defs);
  if (!found) fail(`no response type for ${method} (tried ${candidates.join(", ")}): add it to responseOverrides`);
  return found;
}

function refName(ref) {
  return typeof ref === "string" ? ref.replace(/^#\/definitions\//, "") : undefined;
}

/** Reduces the whole schema to the methods of subset.json and every type they reach. */
function reduceSchema(defs, subset) {
  const tables = {
    clientRequests: methodTable(defs, "ClientRequest"),
    clientNotifications: methodTable(defs, "ClientNotification"),
    serverRequests: methodTable(defs, "ServerRequest"),
    serverNotifications: methodTable(defs, "ServerNotification"),
  };
  const doc = { codex: subset.codex, experimental: subset.experimental, source: "codex app-server generate-json-schema" };
  const roots = [];
  for (const kind of Object.keys(tables)) {
    doc[kind] = {};
    for (const method of subset[kind]) {
      const entry = tables[kind][method];
      if (!entry) fail(`${method} is not a ${kind} method of codex ${subset.codex} any more (subset.json)`);
      const out = { params: entry.params, paramsRequired: entry.paramsRequired };
      if (kind === "clientRequests" || kind === "serverRequests") {
        out.response = responseType(method, entry, defs, subset.responseOverrides ?? {});
        roots.push(out.response);
      }
      if (entry.params) roots.push(...refsIn(entry.params));
      doc[kind][method] = out;
    }
  }
  const seen = new Set();
  const visit = (name) => {
    if (seen.has(name)) return;
    if (!(name in defs)) fail(`dangling reference to ${name}`);
    seen.add(name);
    for (const r of refsIn(defs[name])) visit(r);
  };
  roots.forEach(visit);
  doc.definitions = {};
  for (const name of [...seen].sort()) doc.definitions[name] = defs[name];
  return doc;
}

function refsIn(node, out = []) {
  if (Array.isArray(node)) node.forEach((n) => refsIn(n, out));
  else if (isObj(node)) {
    for (const [k, v] of Object.entries(node)) {
      if (k === "$ref" && typeof v === "string") out.push(refName(v));
      else refsIn(v, out);
    }
  }
  return out;
}

// ------------------------------------------------------------------------------------ Rust names

const KEYWORDS = new Set(
  ("as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod " +
    "move mut pub ref return self static struct super trait true type unsafe use where while abstract become " +
    "box do final macro override priv typeof unsized virtual yield try gen")
    .split(" "),
);

function pascal(s) {
  const out = String(s)
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((p) => p[0].toUpperCase() + p.slice(1))
    .join("");
  if (!out) return "Empty";
  return /^[0-9]/.test(out) ? `V${out}` : out;
}

/** Variant names of an enum: Pascal case, told apart where two values would collapse into one. */
function variantNames(values) {
  const names = values.map(pascal);
  return names.map((n, i) => {
    if (names.indexOf(n) === i && names.lastIndexOf(n) === i) return n;
    // "openai/form" and "openaiForm": spell the separators out.
    return pascal(String(values[i]).replace(/\//g, " slash ").replace(/\./g, " dot "));
  });
}

function snake(s) {
  let out = String(s)
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .replace(/[^A-Za-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .toLowerCase();
  if (!out) out = "field";
  if (/^[0-9]/.test(out)) out = `f_${out}`;
  if (out === "self" || out === "super" || out === "crate") return `${out}_`;
  return KEYWORDS.has(out) ? `r#${out}` : out;
}

const rustString = (s) => JSON.stringify(s);

/**
 * Doc comments from the schema's descriptions. A fence without a language would be compiled as a
 * doctest, and an indented line read as a code block: fences become `text`, lines are unindented.
 */
function docLines(text, indent = "") {
  if (typeof text !== "string" || !text.trim()) return [];
  const out = [];
  let fenced = false;
  for (const raw of text.replace(/\r\n/g, "\n").trim().split("\n")) {
    const line = raw.trimEnd();
    const fence = /^\s*```(.*)$/.exec(line);
    if (fence) {
      if (!fenced) {
        const lang = fence[1].trim();
        out.push(lang === "" || lang === "rust" ? "```text" : `\`\`\`${lang}`);
      } else out.push("```");
      fenced = !fenced;
      continue;
    }
    out.push(fenced ? line : line.trimStart());
  }
  if (fenced) out.push("```");
  return out.map((l) => `${indent}///${l ? ` ${l}` : ""}`);
}

// ------------------------------------------------------------------------------------ type model

// A type expression: {k: "named", name} | {k: "prim", r} | {k: "vec", of} | {k: "map", of} | {k: "opt", of}
const prim = (r) => ({ k: "prim", r });
const VALUE = prim("serde_json::Value");
const META = new Set(["description", "title", "default", "examples", "$schema", "deprecated", "readOnly", "writeOnly"]);

class Generator {
  constructor(doc) {
    this.doc = doc;
    this.defs = doc.definitions;
    this.items = new Map(); // Rust name → item
    this.uses = new Map(); // Rust name → the property names it is the type of
    this.claimed = new Set(Object.keys(this.defs));
    for (const name of Object.keys(this.defs).sort()) this.define(name, this.defs[name]);
  }

  /**
   * Every string enum is open (`Unrecognized(String)`), even one of a single value: Codex adds
   * reasons and kinds. Except an enum only ever used as an object's `type`: it marks which shape
   * the object is (MCP elicitation's `"string"`, `"number"`/`"integer"`…), and must stay exact for
   * the untagged union around it to tell its shapes apart; that union keeps the unknown ones.
   */
  settleEnums() {
    for (const item of this.items.values()) {
      if (item.kind !== "strenum") continue;
      const uses = this.uses.get(item.name);
      item.open = !(uses && [...uses].every((u) => u === "type"));
    }
  }

  /**
   * An untagged union tries its variants in order, and a fallback takes anything: one that is a
   * variant of another untagged union (MCP elicitation's enum schemas inside its primitive schemas)
   * goes without, or it would swallow the shapes of the variants after it. The outer union's
   * fallback keeps what neither knows.
   */
  settleUnions() {
    for (const item of this.items.values()) if (item.kind === "untagged") item.fallback = true;
    const variantItem = (v) => {
      const t = v.type.k === "box" ? v.type.of : v.type;
      return t.k === "named" ? this.items.get(t.name) : undefined;
    };
    // An object the schema closes (`additionalProperties: false`) is told apart from its siblings
    // by that, as Codex's own decoder does (an untitled enum schema has no `enumNames`): such a
    // struct, when it is a shape of an untagged union, refuses unknown fields, so the union tries
    // the next shape. Everywhere else unknown fields are ignored.
    const close = (item) => {
      if (item?.kind === "struct" && item.closedInSchema) item.denyUnknown = true;
      if (item?.kind === "untagged") item.variants.forEach((v) => close(variantItem(v)));
    };
    for (const item of this.items.values()) {
      if (item.kind !== "untagged") continue;
      for (const v of item.variants) {
        const inner = variantItem(v);
        if (inner?.kind === "untagged") inner.fallback = false;
        close(inner);
      }
    }
  }

  claim(base) {
    let name = pascal(base);
    if (!this.claimed.has(name)) {
      this.claimed.add(name);
      return name;
    }
    for (let n = 2; ; n++) {
      if (!this.claimed.has(`${name}${n}`)) {
        this.claimed.add(`${name}${n}`);
        return `${name}${n}`;
      }
    }
  }

  resolve(schema) {
    let s = schema;
    for (let i = 0; i < 16 && isObj(s) && s.$ref; i++) s = this.defs[refName(s.$ref)];
    return s;
  }

  /** A named definition of the schema. */
  define(name, schema) {
    const alts = schema.oneOf ?? schema.anyOf;
    if (alts) {
      const nonNull = alts.filter((a) => !isNull(a));
      if (nonNull.length !== alts.length) {
        // A nullable named type: an alias of Option over its non-null part.
        const inner = nonNull.length === 1 ? this.typeOf(nonNull[0], `${name}Value`) : this.union(this.claim(`${name}Value`), { ...schema, oneOf: nonNull, anyOf: undefined });
        this.items.set(name, { kind: "alias", name, doc: schema.description, type: { k: "opt", of: inner } });
        return;
      }
      this.union(name, schema);
      return;
    }
    if (isStringEnum(schema)) {
      this.stringEnum(name, schema, schema.enum.map((v) => ({ value: v })));
      return;
    }
    if (isObjectSchema(schema)) {
      if (!hasProps(schema) && isObj(schema.additionalProperties)) {
        const of = this.typeOf(schema.additionalProperties, `${name}Value`);
        this.items.set(name, { kind: "alias", name, doc: schema.description, type: { k: "map", of } });
        return;
      }
      this.struct(name, schema);
      return;
    }
    this.items.set(name, { kind: "alias", name, doc: schema.description, type: this.typeOf(schema, `${name}Value`) });
  }

  /** The Rust type of a schema found inside another, naming new items after `ctx`. */
  typeOf(schema, ctx) {
    if (schema === true || !isObj(schema)) return VALUE;
    if (schema.$ref) return { k: "named", name: refName(schema.$ref) };
    const keys = Object.keys(schema).filter((k) => !META.has(k));
    if (keys.length === 0) return VALUE;
    if (Array.isArray(schema.allOf)) {
      if (schema.allOf.length === 1) return this.typeOf(schema.allOf[0], ctx);
      return VALUE;
    }
    const alts = schema.oneOf ?? schema.anyOf;
    if (alts) {
      const nonNull = alts.filter((a) => !isNull(a));
      if (nonNull.length === 0) return VALUE;
      if (nonNull.length !== alts.length) {
        const inner =
          nonNull.length === 1 && !hasProps(schema)
            ? this.typeOf(nonNull[0], ctx)
            : this.typeOf({ ...schema, oneOf: nonNull, anyOf: undefined }, ctx);
        return { k: "opt", of: inner };
      }
      const name = this.claim(schema.title ?? ctx);
      this.union(name, schema);
      return { k: "named", name };
    }
    if (Array.isArray(schema.type)) {
      const types = schema.type.filter((t) => t !== "null");
      const nullable = types.length !== schema.type.length;
      const inner = types.length === 1 ? this.typeOf({ ...schema, type: types[0] }, ctx) : VALUE;
      return nullable ? { k: "opt", of: inner } : inner;
    }
    if (Array.isArray(schema.enum)) {
      if (!isStringEnum(schema)) return primitive(schema);
      const name = this.claim(schema.title ?? ctx);
      this.stringEnum(name, schema, schema.enum.map((v) => ({ value: v })));
      return { k: "named", name };
    }
    if (isObjectSchema(schema)) {
      if (hasProps(schema)) {
        const name = this.claim(schema.title ?? ctx);
        this.struct(name, schema);
        return { k: "named", name };
      }
      if (isObj(schema.additionalProperties)) return { k: "map", of: this.typeOf(schema.additionalProperties, `${ctx}Value`) };
      return { k: "map", of: VALUE };
    }
    if (schema.type === "array") return { k: "vec", of: schema.items ? this.typeOf(schema.items, `${ctx}Item`) : VALUE };
    return primitive(schema);
  }

  /**
   * An object: a struct whose optional fields are `Option`s, unknown fields ignored. An object that
   * also lists alternative shapes (Codex's flattened enums: an image item has either `fileId` or
   * `result`) takes their properties as optional fields: whichever shape comes, it decodes.
   */
  struct(name, schema, extra = {}) {
    const required = new Set([...(schema.required ?? []), ...(extra.required ?? [])]);
    const props = { ...(extra.properties ?? {}), ...(schema.properties ?? {}) };
    for (const alt of (schema.oneOf ?? schema.anyOf ?? []).filter((a) => !isNull(a)).map((a) => this.resolve(a))) {
      for (const [json, propSchema] of Object.entries(alt?.properties ?? {})) {
        if (json in props) continue;
        const note = "Set in one of the alternative shapes of this object (see its description).";
        props[json] = { ...propSchema, description: propSchema.description ? `${propSchema.description}\n\n${note}` : note };
      }
    }
    const skip = extra.skip;
    const fields = [];
    const used = new Set();
    for (const [json, propSchema] of Object.entries(props)) {
      if (json === skip) continue;
      let rust = snake(json);
      while (used.has(rust)) rust = `${rust}_`;
      used.add(rust);
      let type = this.typeOf(propSchema, `${name}${pascal(json)}`);
      const target = type.k === "opt" ? type.of : type;
      if (target.k === "named") {
        if (!this.uses.has(target.name)) this.uses.set(target.name, new Set());
        this.uses.get(target.name).add(json);
      }
      const isRequired = required.has(json);
      if (!isRequired && type.k !== "opt") type = { k: "opt", of: type };
      fields.push({ json, rust, type, required: isRequired, doc: propSchema?.description });
    }
    let flatten = null;
    if (schema.additionalProperties === true) flatten = { k: "map", of: VALUE };
    else if (isObj(schema.additionalProperties) && hasProps(schema))
      flatten = { k: "map", of: this.typeOf(schema.additionalProperties, `${name}Value`) };
    const closedInSchema = schema.additionalProperties === false;
    this.items.set(name, { kind: "struct", name, doc: schema.description ?? extra.doc, fields, flatten, closedInSchema });
    return name;
  }

  /** A string enum; two values or more also take an `Unrecognized(String)` for newer values. */
  stringEnum(name, schema, values) {
    const names = variantNames(values.map((v) => v.value));
    if (names.includes(FALLBACK)) fail(`${name}: a value is named ${FALLBACK}, rename the fallback variant`);
    this.items.set(name, {
      kind: "strenum",
      name,
      doc: schema.description,
      variants: values.map((v, i) => ({ name: names[i], value: v.value, doc: v.doc })),
      open: true, // settled by settleEnums once every use is known
    });
    return name;
  }

  /** oneOf / anyOf: string enum, internally tagged, externally tagged, or untagged (in that order). */
  union(name, schema) {
    const alts = (schema.oneOf ?? schema.anyOf).filter((a) => !isNull(a));
    const resolved = alts.map((a) => this.resolve(a));
    const doc = schema.description;

    // 1. Only string values.
    if (resolved.every((a) => isStringEnum(a)) && alts.every((a) => !a.$ref)) {
      const values = [];
      for (const a of alts) for (const v of a.enum) values.push({ value: v, doc: a.enum.length === 1 ? a.description : undefined });
      return this.stringEnum(name, schema, values);
    }

    // 2. Objects told apart by one property of a single value each: `#[serde(tag = …)]`.
    const sharedProps = schema.properties ?? {};
    const sharedRequired = schema.required ?? [];
    if (alts.every((a) => !a.$ref && isObjectSchema(a) && hasProps(a))) {
      const tag = findTag(alts);
      if (tag) {
        const values = alts.map((a) => a.properties[tag].enum?.[0] ?? a.properties[tag].const);
        const names = variantNames(values);
        if (names.includes(FALLBACK)) fail(`${name}: a variant is named ${FALLBACK}, rename the fallback variant`);
        const variants = alts.map((a, i) => {
          const structName = this.claim(a.title ?? `${name}${names[i]}`);
          this.struct(structName, a, { properties: sharedProps, required: sharedRequired, skip: tag, doc: a.description });
          const it = this.items.get(structName);
          if (!it.fields.length && !it.flatten) {
            // Nothing but the tag (`{"type": "idle"}`): a unit variant, which ignores other fields too.
            this.items.delete(structName);
            return { name: names[i], value: values[i], doc: a.description };
          }
          return { name: names[i], value: values[i], type: { k: "named", name: structName }, doc: a.description };
        });
        this.items.set(name, { kind: "tagged", name, doc, tag, variants });
        return name;
      }
    }
    // An object with properties of its own and alternative shapes: one struct (see `struct`).
    if (hasProps(schema)) return this.struct(name, schema);

    // 3. Strings and single-key objects: serde's default, externally tagged representation. An
    //    alternative of no particular shape (CodexErrorInfo's `["string", "object"]`) is what the
    //    fallback variant already takes.
    const external = alts.filter((a) => !isCatchAll(a));
    if (
      !alts.some((a) => a.$ref) &&
      external.some((a) => isSingleKeyObject(a)) &&
      external.every((a) => isStringEnum(a) || isSingleKeyObject(a))
    ) {
      const entries = [];
      for (const a of external) {
        if (isStringEnum(a)) for (const v of a.enum) entries.push({ value: v, unit: true, doc: a.enum.length === 1 ? a.description : undefined });
        else {
          const key = Object.keys(a.properties)[0];
          entries.push({ value: key, alt: a, doc: a.description ?? a.properties[key].description });
        }
      }
      const names = variantNames(entries.map((e) => e.value));
      if (names.includes(FALLBACK)) fail(`${name}: a variant is named ${FALLBACK}, rename the fallback variant`);
      const variants = entries.map((e, i) => {
        if (e.unit) return { name: names[i], value: e.value, doc: e.doc };
        const inner = e.alt.properties[e.value];
        return { name: names[i], value: e.value, doc: e.doc, type: this.typeOf(inner, e.alt.title ?? `${name}${names[i]}`) };
      });
      this.items.set(name, { kind: "external", name, doc, variants });
      return name;
    }

    // 4. Anything else: tried in order (`#[serde(untagged)]`). Variants named after their types,
    //    less the words they all share (McpElicitationStringSchema, …NumberSchema → String, Number).
    const used = new Set();
    const short = shortNames(alts.filter((a) => a.$ref).map((a) => refName(a.$ref)));
    const variants = alts.map((a, i) => {
      let vname = a.$ref ? short.get(refName(a.$ref)) : a.title ? pascal(a.title) : untaggedName(a);
      while (used.has(vname)) vname = `${vname}${i}`;
      used.add(vname);
      return { name: vname, type: this.typeOf(a, `${name}${vname}`), doc: a.description };
    });
    this.items.set(name, { kind: "untagged", name, doc, variants });
    return name;
  }
}

function isNull(a) {
  return isObj(a) && (a.type === "null" || (Array.isArray(a.type) && a.type.length === 1 && a.type[0] === "null"));
}
function isObjectSchema(s) {
  return isObj(s) && (s.type === "object" || (Array.isArray(s.type) && s.type.includes("object") && s.type.length === 1) || (!s.type && isObj(s.properties)));
}
function hasProps(s) {
  return isObj(s?.properties) && Object.keys(s.properties).length > 0;
}
function isStringEnum(s) {
  return isObj(s) && Array.isArray(s.enum) && s.enum.every((v) => typeof v === "string") && (!s.type || s.type === "string");
}
function isSingleKeyObject(s) {
  return isObjectSchema(s) && hasProps(s) && Object.keys(s.properties).length === 1 && (s.required ?? []).includes(Object.keys(s.properties)[0]);
}
function isCatchAll(s) {
  return isObj(s) && !s.$ref && !s.enum && !hasProps(s) && Array.isArray(s.type) && s.type.length > 1;
}
function singleValue(p) {
  if (!isObj(p)) return undefined;
  if (Array.isArray(p.enum) && p.enum.length === 1) return p.enum[0];
  if ("const" in p) return p.const;
  return undefined;
}
/** The property that tells the alternatives apart: required, one string value each, all different. */
function findTag(alts) {
  const candidates = Object.keys(alts[0].properties).filter((k) =>
    alts.every((a) => (a.required ?? []).includes(k) && typeof singleValue(a.properties?.[k]) === "string"),
  );
  const distinct = candidates.filter((k) => new Set(alts.map((a) => singleValue(a.properties[k]))).size === alts.length);
  for (const preferred of ["type", "kind", "mode", "method"]) if (distinct.includes(preferred)) return preferred;
  return distinct[0];
}
/** Type names less the Pascal-case words they all begin and end with, when that leaves each a name. */
function shortNames(names) {
  const out = new Map(names.map((n) => [n, n]));
  if (names.length < 2) return out;
  const words = names.map((n) => n.match(/[A-Z][a-z0-9]*|[a-z0-9]+/g) ?? [n]);
  let head = 0;
  while (words.every((w) => w.length > head && w[head] === words[0][head])) head++;
  let tail = 0;
  while (words.every((w) => w.length > head + tail && w[w.length - 1 - tail] === words[0][words[0].length - 1 - tail])) tail++;
  const shortened = words.map((w) => w.slice(head, w.length - tail).join(""));
  if (shortened.some((s) => !s || /^[0-9]/.test(s)) || new Set(shortened).size !== shortened.length) return out;
  names.forEach((n, i) => out.set(n, shortened[i]));
  return out;
}

function untaggedName(a) {
  const t = Array.isArray(a.type) ? a.type.filter((x) => x !== "null")[0] : a.type;
  return { string: "String", integer: "Integer", number: "Number", boolean: "Bool", array: "Array", object: "Object" }[t] ?? "Value";
}
function primitive(schema) {
  switch (schema.type) {
    case "string":
      return prim("String");
    case "boolean":
      return prim("bool");
    case "integer":
      // Widened to 64 bits: a counter that outgrows a u32 in a later release still decodes.
      return prim(/^uint/.test(schema.format ?? "") || schema.minimum >= 0 ? "u64" : "i64");
    case "number":
      return prim("f64");
    default:
      return VALUE;
  }
}

// ------------------------------------------------------------------------------------ Rust output

/** Boxes what an item holds of itself (by value, through Option): a recursive type has a size. */
function markRecursion(items) {
  const edges = new Map();
  const byValue = (t) => (t.k === "named" ? [t.name] : t.k === "opt" ? byValue(t.of) : []);
  for (const item of items.values()) {
    const out = [];
    if (item.kind === "struct") item.fields.forEach((f) => out.push(...byValue(f.type)));
    if (item.kind === "alias") out.push(...byValue(item.type));
    if (["tagged", "external", "untagged"].includes(item.kind)) item.variants.forEach((v) => v.type && out.push(...byValue(v.type)));
    edges.set(item.name, out.filter((n) => items.has(n)));
  }
  // Tarjan's strongly connected components.
  let index = 0;
  const idx = new Map();
  const low = new Map();
  const stack = [];
  const on = new Set();
  const comp = new Map();
  const strong = (v) => {
    idx.set(v, index);
    low.set(v, index++);
    stack.push(v);
    on.add(v);
    for (const w of edges.get(v)) {
      if (!idx.has(w)) {
        strong(w);
        low.set(v, Math.min(low.get(v), low.get(w)));
      } else if (on.has(w)) low.set(v, Math.min(low.get(v), idx.get(w)));
    }
    if (low.get(v) === idx.get(v)) {
      let w;
      do {
        w = stack.pop();
        on.delete(w);
        comp.set(w, v);
      } while (w !== v);
    }
  };
  for (const v of edges.keys()) if (!idx.has(v)) strong(v);
  const boxIn = (owner, t) => {
    if (t.k === "named" && comp.get(t.name) === comp.get(owner)) return { k: "box", of: t };
    if (t.k === "opt") return { k: "opt", of: boxIn(owner, t.of) };
    return t;
  };
  for (const item of items.values()) {
    if (item.kind === "struct") item.fields.forEach((f) => (f.type = boxIn(item.name, f.type)));
    if (["tagged", "external", "untagged"].includes(item.kind))
      item.variants.forEach((v) => v.type && (v.type = boxIn(item.name, v.type)));
  }
}

/** Which structs can derive Default: every field can (Option, Vec, maps, primitives, such structs). */
function markDefault(items) {
  const ok = (t) => {
    switch (t.k) {
      case "opt":
      case "vec":
      case "map":
      case "prim":
        return true;
      case "box":
        return ok(t.of);
      case "named": {
        const it = items.get(t.name);
        return it?.kind === "struct" ? it.default === true : it?.kind === "alias" ? ok(it.type) : false;
      }
    }
    return false;
  };
  for (const item of items.values()) if (item.kind === "struct") item.default = false;
  for (let changed = true; changed; ) {
    changed = false;
    for (const item of items.values()) {
      if (item.kind !== "struct" || item.default) continue;
      if (item.fields.every((f) => ok(f.type))) {
        item.default = true;
        changed = true;
      }
    }
  }
}

function render(t) {
  switch (t.k) {
    case "named":
      return t.name;
    case "prim":
      return t.r;
    case "vec":
      return `Vec<${render(t.of)}>`;
    case "map":
      return `std::collections::BTreeMap<String, ${render(t.of)}>`;
    case "opt":
      return `Option<${render(t.of)}>`;
    case "box":
      return `Box<${render(t.of)}>`;
  }
  throw new Error(`unknown type ${JSON.stringify(t)}`);
}

function renderItem(item) {
  const out = [...docLines(item.doc)];
  const derive = (extra = []) => `#[derive(Debug, Clone, PartialEq, ${extra.join(", ")}${extra.length ? ", " : ""}Serialize, Deserialize)]`;
  switch (item.kind) {
    case "alias":
      out.push(`pub type ${item.name} = ${render(item.type)};`);
      break;
    case "struct": {
      out.push(derive(item.default ? ["Default"] : []));
      if (item.denyUnknown) out.push("#[serde(deny_unknown_fields)]");
      if (!item.fields.length && !item.flatten) {
        out.push(`pub struct ${item.name} {}`);
        break;
      }
      out.push(`pub struct ${item.name} {`);
      for (const f of item.fields) {
        out.push(...docLines(f.doc, "    "));
        const attrs = [];
        if (f.rust.replace(/^r#/, "") !== f.json) attrs.push(`rename = ${rustString(f.json)}`);
        if (f.type.k === "opt") {
          attrs.push("default");
          if (!f.required) attrs.push(`skip_serializing_if = "Option::is_none"`);
        }
        if (attrs.length) out.push(`    #[serde(${attrs.join(", ")})]`);
        out.push(`    pub ${f.rust}: ${render(f.type)},`);
      }
      if (item.flatten) {
        out.push("    /// The fields the schema leaves open.");
        out.push("    #[serde(flatten)]");
        out.push(`    pub extra: ${render(item.flatten)},`);
      }
      out.push("}");
      break;
    }
    case "strenum": {
      out.push(item.open ? derive(["Eq", "Hash"]) : derive(["Copy", "Eq", "Hash"]));
      out.push(`pub enum ${item.name} {`);
      for (const v of item.variants) {
        out.push(...docLines(v.doc, "    "));
        out.push(`    #[serde(rename = ${rustString(v.value)})]`);
        out.push(`    ${v.name},`);
      }
      if (item.open) {
        out.push("    /// A value this version of the protocol does not know, kept as it came.");
        out.push("    #[serde(untagged)]");
        out.push(`    ${FALLBACK}(String),`);
      }
      out.push("}");
      break;
    }
    case "tagged":
    case "external": {
      out.push(derive());
      if (item.kind === "tagged") out.push(`#[serde(tag = ${rustString(item.tag)})]`);
      out.push(`pub enum ${item.name} {`);
      for (const v of item.variants) {
        out.push(...docLines(v.doc, "    "));
        out.push(`    #[serde(rename = ${rustString(v.value)})]`);
        out.push(v.type ? `    ${v.name}(${render(v.type)}),` : `    ${v.name},`);
      }
      out.push("    /// A variant this version of the protocol does not know (or could not read), kept as it came.");
      out.push("    #[serde(untagged)]");
      out.push(`    ${FALLBACK}(serde_json::Value),`);
      out.push("}");
      break;
    }
    case "untagged": {
      out.push(derive());
      out.push("#[serde(untagged)]");
      out.push(`pub enum ${item.name} {`);
      for (const v of item.variants) {
        out.push(...docLines(v.doc, "    "));
        out.push(`    ${v.name}(${render(v.type)}),`);
      }
      if (item.fallback) {
        out.push("    /// A shape this version of the protocol does not know, kept as it came.");
        out.push(`    ${FALLBACK}(serde_json::Value),`);
      }
      out.push("}");
      break;
    }
  }
  return out.join("\n");
}

/** The Rust type of a method's params (`()` for none, `Option<…>` when they may be left out). */
function paramsType(gen, method, entry) {
  if (!entry.params) return "()";
  const t = gen.typeOf(entry.params, `${pascalMethod(method)}Params`);
  if (!entry.paramsRequired && t.k !== "opt") return `Option<${render(t)}>`;
  return render(t);
}

function renderMethods(gen, doc) {
  const out = [];
  const marker = (kind, method, body) => {
    const name = pascalMethod(method);
    return [`    /// \`${method}\``, "    #[derive(Debug, Clone, Copy, PartialEq, Eq)]", `    pub struct ${name};`, `    impl super::${kind} for ${name} {`, ...body, "    }"].join("\n");
  };
  out.push(
    `/// A request Hangar sends to the app-server: its method, its params and the type of the answer.
pub trait ClientRequest {
    /// The JSON-RPC method.
    const METHOD: &'static str;
    /// What goes in \`params\`.
    type Params: Serialize + serde::de::DeserializeOwned;
    /// What comes back in \`result\`.
    type Response: Serialize + serde::de::DeserializeOwned;
}

/// A notification Hangar sends to the app-server.
pub trait ClientNotification {
    /// The JSON-RPC method.
    const METHOD: &'static str;
    /// What goes in \`params\` (\`()\`: none).
    type Params: Serialize + serde::de::DeserializeOwned;
}

/// A request the app-server sends to Hangar: its params and the answer it waits for.
pub trait ServerRequestMethod {
    /// The JSON-RPC method.
    const METHOD: &'static str;
    /// What comes in \`params\`.
    type Params: Serialize + serde::de::DeserializeOwned;
    /// What goes back in \`result\`.
    type Response: Serialize + serde::de::DeserializeOwned;
}`,
  );
  const rq = Object.entries(doc.clientRequests).map(([m, e]) =>
    marker("ClientRequest", m, [
      `        const METHOD: &'static str = ${rustString(m)};`,
      `        type Params = ${qualify(paramsType(gen, m, e))};`,
      `        type Response = super::${e.response};`,
    ]),
  );
  out.push(`/// The requests Hangar sends, one marker type each (scripts/codex-protocol/subset.json).\npub mod requests {\n${rq.join("\n\n")}\n}`);
  const cn = Object.entries(doc.clientNotifications).map(([m, e]) =>
    marker("ClientNotification", m, [`        const METHOD: &'static str = ${rustString(m)};`, `        type Params = ${qualify(paramsType(gen, m, e))};`]),
  );
  out.push(`/// The notifications Hangar sends.\npub mod notifications {\n${cn.join("\n\n")}\n}`);
  const sr = Object.entries(doc.serverRequests).map(([m, e]) =>
    marker("ServerRequestMethod", m, [
      `        const METHOD: &'static str = ${rustString(m)};`,
      `        type Params = ${qualify(paramsType(gen, m, e))};`,
      `        type Response = super::${e.response};`,
    ]),
  );
  out.push(`/// The requests the app-server sends that Hangar answers, one marker type each.\npub mod server_requests {\n${sr.join("\n\n")}\n}`);

  const dispatch = (name, what, table) => {
    const lines = [
      `/// ${what}, by method, read from the whole JSON-RPC message (\`method\` and \`params\`;`,
      "/// other keys such as `id` are ignored). A method outside Hangar's subset, or one whose params",
      `/// this version could not read, is \`${FALLBACK}\` with the message as it came: check its method`,
      `/// against [\`${name}::METHODS\`] to tell the two apart.`,
      derive(),
      `#[serde(tag = "method", content = "params")]`,
      `pub enum ${name} {`,
    ];
    for (const [m, e] of Object.entries(table)) {
      if (e.params && !e.paramsRequired) fail(`${m}: optional params in a dispatch enum are not supported by the generator`);
      lines.push(`    /// \`${m}\``);
      lines.push(`    #[serde(rename = ${rustString(m)})]`);
      lines.push(e.params ? `    ${pascalMethod(m)}(${paramsType(gen, m, e)}),` : `    ${pascalMethod(m)},`);
    }
    lines.push("    /// Not in the subset, or not readable by this version: the message as it came.");
    lines.push("    #[serde(untagged)]");
    lines.push(`    ${FALLBACK}(serde_json::Value),`);
    lines.push("}");
    lines.push("");
    lines.push(`impl ${name} {`);
    lines.push("    /// Every method this enum reads.");
    lines.push(`    pub const METHODS: &'static [&'static str] = &[${Object.keys(table).map(rustString).join(", ")}];`);
    lines.push("");
    lines.push(`    /// The method, also of an \`${FALLBACK}\` message (empty when it has none).`);
    lines.push("    pub fn method(&self) -> &str {");
    lines.push("        match self {");
    for (const [m, e] of Object.entries(table)) lines.push(`            Self::${pascalMethod(m)}${e.params ? "(_)" : ""} => ${rustString(m)},`);
    lines.push(`            Self::${FALLBACK}(message) => message.get("method").and_then(serde_json::Value::as_str).unwrap_or(""),`);
    lines.push("        }");
    lines.push("    }");
    lines.push("}");
    return lines.join("\n");
  };
  out.push(dispatch("ServerNotification", "A notification from the app-server", doc.serverNotifications));
  out.push(dispatch("ServerRequest", "A request from the app-server", doc.serverRequests));

  // For the tests: decode by the schema's type name, so they follow the subset without a list of their own.
  const names = [...Object.keys(doc.definitions), "ServerNotification", "ServerRequest"].sort();
  out.push(
    [
      "/// Decoding by the schema's type name, for the tests (codex_app/tests.rs).",
      "#[cfg(test)]",
      "pub(crate) mod checks {",
      "    /// Decodes `value` as the type the schema calls `name`, and gives it back encoded again with",
      "    /// its `Debug` text; `None` when no type has that name.",
      "    pub(crate) fn decode_named(name: &str, value: serde_json::Value) -> Option<Result<(serde_json::Value, String), String>> {",
      "        Some(match name {",
      ...names.map((n) => `            ${rustString(n)} => round_trip::<super::${n}>(value),`),
      "            _ => return None,",
      "        })",
      "    }",
      "",
      "    fn round_trip<T>(value: serde_json::Value) -> Result<(serde_json::Value, String), String>",
      "    where",
      "        T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug,",
      "    {",
      "        let typed: T = serde_json::from_value(value).map_err(|e| e.to_string())?;",
      "        let back = serde_json::to_value(&typed).map_err(|e| e.to_string())?;",
      '        Ok((back, format!("{typed:?}")))',
      "    }",
      "}",
    ].join("\n"),
  );
  return out;
}

// ------------------------------------------------------------------------------------ samples

/**
 * JSON instances of every type of the subset, written from the schema alone, never from the Rust
 * types: each value of an enum, each alternative of a union, each object with all its properties
 * and with the required ones only, each notification and server request as a message. The tests
 * decode every one with its Rust type, find no `Unrecognized` in it, and encode it back to the same
 * JSON. A named type met inside another is sampled with its required properties only: it has its
 * own cases.
 */
class Sampler {
  constructor(defs) {
    this.defs = defs;
  }

  resolve(schema) {
    let s = schema;
    for (let i = 0; i < 16 && isObj(s) && s.$ref; i++) s = this.defs[refName(s.$ref)];
    return s;
  }

  /** A value of `schema`, or undefined when only an infinite one would do. */
  sample(schema, full, stack) {
    if (schema === true || !isObj(schema)) return { any: 1 };
    if (schema.$ref) {
      const name = refName(schema.$ref);
      if (stack.includes(name)) return undefined;
      return this.sample(this.defs[name], false, [...stack, name]);
    }
    if (Object.keys(schema).every((k) => META.has(k))) return { any: 1 };
    if (Array.isArray(schema.allOf)) return schema.allOf.length === 1 ? this.sample(schema.allOf[0], full, stack) : { any: 1 };
    const alts = schema.oneOf ?? schema.anyOf;
    if (alts) {
      for (const alt of alts.filter((a) => !isNull(a))) {
        const v = this.alternative(schema, alt, full, stack);
        if (v !== undefined) return v;
      }
      return alts.some(isNull) ? null : undefined;
    }
    if (Array.isArray(schema.type)) {
      const types = schema.type.filter((t) => t !== "null");
      if (types.length === 1) return this.sample({ ...schema, type: types[0] }, full, stack);
      return types.length ? { any: 1 } : null;
    }
    if (Array.isArray(schema.enum)) return schema.enum[0];
    if ("const" in schema) return schema.const;
    if (isObjectSchema(schema)) return this.object(schema, full, stack);
    switch (schema.type) {
      case "string":
        return "text";
      case "integer":
        return 7;
      case "number":
        return 2.5;
      case "boolean":
        return true;
      case "array": {
        if (!full) return [];
        const v = schema.items ? this.sample(schema.items, false, stack) : { any: 1 };
        return v === undefined ? [] : [v];
      }
      case "null":
        return null;
    }
    return { any: 1 };
  }

  /** One alternative of a union, with the properties the union's object shares with all of them. */
  alternative(parent, alt, full, stack) {
    if (!hasProps(parent)) return this.sample(alt, full, stack);
    const a = this.resolve(alt) ?? {};
    return this.object(
      {
        ...a,
        properties: { ...parent.properties, ...(a.properties ?? {}) },
        required: [...(parent.required ?? []), ...(a.required ?? [])],
      },
      full,
      stack,
    );
  }

  object(schema, full, stack) {
    const props = { ...(schema.properties ?? {}) };
    const required = new Set(schema.required ?? []);
    // An object with alternative shapes: the first one, or all of them (`anyOf`) when full.
    const alts = (schema.oneOf ?? schema.anyOf ?? []).filter((a) => !isNull(a));
    for (const alt of full && schema.anyOf ? alts : alts.slice(0, 1)) {
      const a = this.resolve(alt) ?? {};
      for (const [k, p] of Object.entries(a.properties ?? {})) if (!(k in props)) props[k] = p;
      for (const k of a.required ?? []) required.add(k);
    }
    const out = {};
    for (const [k, p] of Object.entries(props)) {
      const isRequired = required.has(k);
      if (!isRequired && !full) continue;
      let v = !full && nullable(p, this) ? null : this.sample(p, full, stack);
      if (v === undefined) {
        if (!isRequired) continue;
        if (!nullable(p, this)) return undefined;
        v = null;
      }
      out[k] = v;
    }
    if (full && !hasProps(schema) && isObj(schema.additionalProperties)) {
      const v = this.sample(schema.additionalProperties, false, stack);
      if (v !== undefined) out.key = v;
    }
    return out;
  }

  /** The cases of one named type. */
  cases(name) {
    const schema = this.defs[name];
    const stack = [name];
    const out = [];
    const push = (label, json) => {
      if (json !== undefined && !out.some((c) => JSON.stringify(c.json) === JSON.stringify(json))) out.push({ label, json });
    };
    const alts = schema.oneOf ?? schema.anyOf;
    if (alts) {
      alts.forEach((alt, i) => {
        if (isNull(alt)) return push("null", null);
        // "Anything of these JSON types" (CodexErrorInfo's last): no shape to write, the fallback's.
        if (isCatchAll(alt)) return;
        const a = this.resolve(alt);
        if (isStringEnum(a) && !hasProps(schema)) return a.enum.forEach((v) => push(`value ${v}`, v));
        const label = a?.title ?? alt.$ref?.replace(/^#\/definitions\//, "") ?? `alternative ${i + 1}`;
        push(`${label}, every property`, this.alternative(schema, alt, true, stack));
        push(`${label}, required properties`, this.alternative(schema, alt, false, stack));
      });
    } else if (isStringEnum(schema)) schema.enum.forEach((v) => push(`value ${v}`, v));
    else if (isObjectSchema(schema)) {
      push("every property", this.object(schema, true, stack));
      push("required properties", this.object(schema, false, stack));
    } else push("value", this.sample(schema, true, stack));
    if (!out.length) fail(`no JSON instance of ${name} could be written`);
    return out;
  }

  /** A whole message of each notification and server request, params with every property. */
  messages(table) {
    return Object.entries(table).map(([method, e]) => {
      const params = e.params ? this.sample(this.resolve(e.params), true, [refName(e.params.$ref)]) : undefined;
      return { label: method, json: params === undefined ? { method } : { method, params } };
    });
  }
}

function nullable(p, sampler) {
  const s = sampler.resolve(p);
  if (!isObj(s)) return false;
  if (s.type === "null" || (Array.isArray(s.type) && s.type.includes("null"))) return true;
  return (s.oneOf ?? s.anyOf ?? []).some(isNull);
}

function samplesFile(doc) {
  const sampler = new Sampler(doc.definitions);
  const lines = [];
  for (const name of Object.keys(doc.definitions).sort())
    for (const c of sampler.cases(name)) lines.push(JSON.stringify({ type: name, case: c.label, json: c.json }));
  for (const [type, table] of [
    ["ServerNotification", doc.serverNotifications],
    ["ServerRequest", doc.serverRequests],
  ])
    for (const c of sampler.messages(table)) lines.push(JSON.stringify({ type, case: c.label, json: c.json }));
  return `${lines.join("\n")}\n`;
}

/** A type expression written inside one of the method modules, where the types are in `super`. */
function qualify(t) {
  // Not a path's later segment (`serde_json::Value`, `std::collections::BTreeMap`), nor std's own.
  return t.replace(/(?<![:\w])([A-Z][A-Za-z0-9]*)\b/g, (m) => (["Option", "Vec", "String", "Box"].includes(m) ? m : `super::${m}`));
}

const derive = (extra = []) => `#[derive(Debug, Clone, PartialEq, ${extra.join(", ")}${extra.length ? ", " : ""}Serialize, Deserialize)]`;

function generateRust(doc) {
  const gen = new Generator(doc);
  const methods = renderMethods(gen, doc); // may add inline types
  gen.settleEnums();
  gen.settleUnions();
  markRecursion(gen.items);
  markDefault(gen.items);
  const header = [
    "// @generated by scripts/codex-protocol/generate.mjs: do not edit. Change subset.json or the",
    "// generator and run it again (scripts/codex-protocol/README.md).",
    "",
    `//! The Codex app-server protocol (v2) as Hangar uses it: the types of Codex ${doc.codex}, from`,
    "//! `codex app-server generate-json-schema` (its stable surface), reduced to the methods listed in",
    "//! scripts/codex-protocol/subset.json.",
    "//!",
    "//! Decoding is lenient: unknown fields are ignored, and enums and unions keep what they do not",
    `//! know in an \`${FALLBACK}\` variant instead of failing. A field the schema requires is required.`,
    "//! One exception, which never fails a message either: the shapes of an untagged union that the",
    "//! schema closes (MCP elicitation's field schemas) refuse unknown fields, as Codex's own decoder",
    `//! does, to be told apart; one that matches none is the union's \`${FALLBACK}\`.`,
    "",
    "#![allow(clippy::all)]",
    "",
    "use serde::{Deserialize, Serialize};",
    "",
    "/// The Codex release the types were generated from.",
    `pub const CODEX_VERSION: &str = ${rustString(doc.codex)};`,
  ];
  const body = [...gen.items.values()].sort((a, b) => a.name.localeCompare(b.name)).map(renderItem);
  return `${[header.join("\n"), ...methods, ...body].join("\n\n")}\n`;
}

function rustfmt(source) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-protocol-fmt-"));
  const file = path.join(dir, "generated.rs");
  fs.writeFileSync(file, source);
  try {
    execFileSync("rustfmt", ["--edition", "2021", file], { stdio: ["ignore", "inherit", "inherit"] });
  } catch (e) {
    fail(`rustfmt failed (${e.message}); it is needed to write a file cargo fmt --check accepts`);
  }
  const out = fs.readFileSync(file, "utf8");
  fs.rmSync(dir, { recursive: true, force: true });
  return out;
}

// ------------------------------------------------------------------------------------ main

function main() {
  const args = parseArgs(process.argv.slice(2));
  const subset = readJson(SUBSET);
  let doc;
  if (args.schema || args.codex) {
    const dir = args.codex ? schemaFromCodex(args.codex, subset) : path.resolve(args.schema);
    doc = reduceSchema(flattenBundle(dir, subset), subset);
  } else {
    doc = readJson(SCHEMA_SUBSET);
    if (doc.codex !== subset.codex) fail(`schema.subset.json is codex ${doc.codex}, subset.json pins ${subset.codex}: run with --schema`);
    for (const kind of ["clientRequests", "clientNotifications", "serverRequests", "serverNotifications"]) {
      const have = Object.keys(doc[kind]);
      if (JSON.stringify(have) !== JSON.stringify(subset[kind]))
        fail(`subset.json changed the ${kind} since schema.subset.json was written: run with --schema`);
    }
  }
  const outputs = [
    [SCHEMA_SUBSET, `${JSON.stringify(doc, null, 2)}\n`],
    [OUT, rustfmt(generateRust(doc))],
    [SAMPLES, samplesFile(doc)],
  ];
  let stale = 0;
  for (const [file, content] of outputs) {
    const current = fs.existsSync(file) ? fs.readFileSync(file, "utf8") : null;
    if (current === content) continue;
    if (args.check) {
      console.error(`codex-protocol: ${path.relative(ROOT, file)} is not what the generator writes`);
      stale++;
    } else {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, content);
      console.log(`wrote ${path.relative(ROOT, file)}`);
    }
  }
  const count = Object.keys(doc.definitions).length;
  console.log(`codex ${doc.codex}: ${count} schema types, ${Object.keys(doc.clientRequests).length} requests, ${Object.keys(doc.serverNotifications).length} notifications, ${Object.keys(doc.serverRequests).length} server requests`);
  if (stale) process.exit(1);
}

main();
