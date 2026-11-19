// node --test scripts/codex-protocol/   (no install: Node's own test runner)
//
// The schema watch's diff on two small schemas written by hand, in the shape `codex app-server
// generate-json-schema` writes them, and on the checked-in reduced schema. No Codex is run.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { diffSchemas, render } from "./diff-schema.mjs";
import { ProtocolError, flattenBundle, readJson } from "./generate.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SCRIPT = path.join(HERE, "diff-schema.mjs");

// ------------------------------------------------------------------------------ hand-made schemas

const ref = (name) => ({ $ref: `#/definitions/v2/${name}` });
const str = { type: "string" };
const strOrNull = { type: ["string", "null"] };

/** One alternative of a method union, as the bundle writes it. */
function methodAlt(method, params) {
  const properties = { method: { enum: [method], title: `${method}Method`, type: "string" } };
  const required = ["method"];
  if (params) {
    properties.params = ref(params);
    required.push("params");
  }
  return { properties, required, title: `${method}Request`, type: "object" };
}

/** A schema bundle: methods by kind (`{ method: params type | null }`) and the types they use. */
function bundle({ clientRequests = {}, clientNotifications = {}, serverRequests = {}, serverNotifications = {}, types }) {
  const union = (methods) => ({ oneOf: Object.entries(methods).map(([m, p]) => methodAlt(m, p)) });
  return {
    definitions: {
      ClientRequest: union(clientRequests),
      ClientNotification: union(clientNotifications),
      ServerRequest: union(serverRequests),
      ServerNotification: union(serverNotifications),
      v2: types,
    },
  };
}

const SUBSET = {
  codex: "1.0.0",
  experimental: false,
  clientRequests: ["thing/get", "thing/drop"],
  clientNotifications: ["initialized"],
  serverRequests: [],
  serverNotifications: ["thing/changed"],
};

/** Codex 1.0.0, the pinned one. */
function pinned() {
  return bundle({
    clientRequests: { "thing/get": "ThingGetParams", "thing/drop": "ThingDropParams" },
    clientNotifications: { initialized: null },
    serverNotifications: { "thing/changed": "ThingChangedNotification", "thing/legacy": "ThingLegacyNotification" },
    types: {
      ThingGetParams: {
        properties: { id: str, legacy: str, mode: ref("Mode"), note: strOrNull },
        required: ["id"],
        type: "object",
      },
      ThingGetResponse: { properties: { thing: ref("Thing") }, required: ["thing"], type: "object" },
      ThingDropParams: { properties: { id: str }, required: ["id"], type: "object" },
      ThingDropResponse: { type: "object" },
      ThingChangedNotification: {
        properties: { reason: ref("Reason"), thing: ref("Thing") },
        required: ["thing"],
        type: "object",
      },
      Mode: { enum: ["fast", "slow"], type: "string" },
      Reason: { oneOf: [{ enum: ["created"], type: "string" }, { enum: ["deleted"], type: "string" }] },
      Thing: {
        oneOf: [
          {
            properties: { size: { type: "integer" }, type: { enum: ["box"], type: "string" } },
            required: ["size", "type"],
            title: "BoxThing",
            type: "object",
          },
          {
            properties: { radius: { type: "number" }, type: { enum: ["ball"], type: "string" } },
            required: ["radius", "type"],
            title: "BallThing",
            type: "object",
          },
        ],
      },
    },
  });
}

/**
 * Codex 1.1.0: thing/drop is gone, ThingGetParams loses `legacy`, gains `limit` and a required
 * `token`, `note` is required and `id` also takes integers, Mode and Reason have a value more,
 * Thing has a new shape (`cube`) and its `box` a new field. Outside the subset, a method comes and
 * one goes. Descriptions are written everywhere: they are not part of what Hangar uses.
 */
function latest() {
  const b = pinned();
  const types = b.definitions.v2;
  b.definitions.ClientRequest.oneOf = [methodAlt("thing/get", "ThingGetParams"), methodAlt("thing/list", "ThingListParams")];
  b.definitions.ServerNotification.oneOf = [
    methodAlt("thing/changed", "ThingChangedNotification"),
    methodAlt("thing/archived", "ThingArchivedNotification"),
  ];
  delete types.ThingDropParams;
  delete types.ThingDropResponse;
  types.ThingGetParams = {
    description: "Which thing, now documented.",
    properties: {
      id: { type: ["string", "integer"] },
      limit: { type: "integer" },
      mode: ref("Mode"),
      note: strOrNull,
      token: str,
    },
    required: ["id", "note", "token"],
    type: "object",
  };
  types.Mode = { description: "How fast.", enum: ["fast", "slow", "medium"], type: "string" };
  types.Reason.oneOf.push({ enum: ["archived"], type: "string" });
  types.ThingChangedNotification.description = "Sent when a thing changes.";
  types.Thing.oneOf[0].properties.color = str;
  types.Thing.oneOf.push({
    properties: { edge: { type: "number" }, type: { enum: ["cube"], type: "string" } },
    required: ["edge", "type"],
    title: "CubeThing",
    type: "object",
  });
  return b;
}

const flat = (b) => flattenFromObject(b);

/** generate.mjs reads a folder; the tests keep the bundle in memory, so write it down once. */
function flattenFromObject(b) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codex-schema-test-"));
  try {
    fs.writeFileSync(path.join(dir, "codex_app_server_protocol.schemas.json"), JSON.stringify(b));
    return flattenBundle(dir);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const rows = ({ changes }) => changes.map((c) => [c.scope, c.path, c.kind, c.detail]);

// ------------------------------------------------------------------------------------------ the diff

test("a subset that did not change gives no change, whatever the descriptions say", () => {
  const b = pinned();
  b.definitions.v2.Mode.description = "Only documentation differs.";
  b.definitions.v2.ThingGetParams.properties.id.description = "So does this.";
  const result = diffSchemas({ pinned: flat(pinned()), latest: flat(b), subset: SUBSET });
  assert.deepEqual(result.changes, []);
  assert.deepEqual(result.notes, []);
});

test("methods gone, fields added, removed or now required, enum values added, shapes added", () => {
  const result = diffSchemas({ pinned: flat(pinned()), latest: flat(latest()), subset: SUBSET });
  assert.deepEqual(rows(result), [
    ["clientRequests thing/drop", "", "method-removed", ""],
    ["type Mode", "", "enum-added", '"medium"'],
    ["type Reason", "", "enum-added", '"archived"'],
    // The new `cube` shape is one change: its own fields are not listed one by one.
    ["type Thing", "", "variant-added", "type=cube"],
    ["type Thing", "<type=box>.color", "field-added", "string"],
    ["type ThingGetParams", ".id", "type-changed", "string -> integer | string"],
    ["type ThingGetParams", ".legacy", "field-removed", ""],
    ["type ThingGetParams", ".limit", "field-added", "integer"],
    ["type ThingGetParams", ".note", "field-required", ""],
    ["type ThingGetParams", ".token", "field-added", "string, required"],
  ]);
});

test("methods outside the subset are noted and never change the subset", () => {
  const result = diffSchemas({ pinned: flat(pinned()), latest: flat(latest()), subset: SUBSET });
  assert.deepEqual(result.notes, [
    { kind: "clientRequests", added: ["thing/list"], removed: [] },
    { kind: "serverNotifications", added: ["thing/archived"], removed: ["thing/legacy"] },
  ]);
  // Only the notes differ: unchanged.
  const b = pinned();
  b.definitions.ServerNotification.oneOf.push(methodAlt("thing/archived", "ThingArchivedNotification"));
  const onlyNotes = diffSchemas({ pinned: flat(pinned()), latest: flat(b), subset: SUBSET });
  assert.deepEqual(onlyNotes.changes, []);
  assert.deepEqual(onlyNotes.notes, [{ kind: "serverNotifications", added: ["thing/archived"], removed: [] }]);
});

test("a method of the subset that no longer has an answer type is a change, not a crash", () => {
  const b = latest();
  delete b.definitions.v2.ThingGetResponse;
  const result = diffSchemas({ pinned: flat(pinned()), latest: flat(b), subset: SUBSET });
  const kinds = result.changes.map((c) => c.kind);
  assert.deepEqual(kinds, ["method-removed", "unreducible"]);
  assert.match(result.changes[1].detail, /no response type for thing\/get/);
});

test("a pinned schema that lacks a method of subset.json is an error, not a diff", () => {
  const b = pinned();
  b.definitions.ClientRequest.oneOf.pop();
  assert.throws(() => diffSchemas({ pinned: flat(b), latest: flat(latest()), subset: SUBSET }), (e) => {
    assert.ok(e instanceof ProtocolError);
    assert.match(e.message, /the pinned schema does not fit subset\.json: thing\/drop is not a clientRequests method/);
    return true;
  });
});

test("the text names both releases, one line per change, and has no backtick", () => {
  const result = diffSchemas({ pinned: flat(pinned()), latest: flat(latest()), subset: SUBSET });
  const text = render(result, { pinned: "codex-cli 1.0.0", latest: "codex-cli 1.1.0" });
  assert.match(text, /pinned {2}codex-cli 1\.0\.0\n {2}latest {2}codex-cli 1\.1\.0\n/);
  assert.match(text, /\nclientRequests thing\/drop\n {2}- method removed\n/);
  assert.match(text, /\ntype ThingGetParams\n {2}~ \.id {2}type changed: string -> integer \| string\n {2}- \.legacy {2}field removed\n {2}\+ \.limit {2}field added: integer\n {2}~ \.note {2}now required\n {2}\+ \.token {2}field added: string, required\n/);
  assert.match(text, /\n10 changes in the subset\.\n/);
  assert.match(text, /serverNotifications added: thing\/archived\n {2}serverNotifications removed: thing\/legacy\n/);
  assert.ok(!text.includes("`"));
  const same = render({ changes: [], notes: [] }, { pinned: "codex-cli 1.0.0", latest: "codex-cli 1.0.0" });
  assert.match(same, /\nThe subset is unchanged\.\n$/);
});

// ------------------------------------------------------------------------- the checked-in schema

/** schema.subset.json (the reduced schema) put back in the shape of a bundle, to read real types. */
function defsFromReduced(doc) {
  const defs = structuredClone(doc.definitions);
  const unions = { clientRequests: "ClientRequest", clientNotifications: "ClientNotification", serverRequests: "ServerRequest", serverNotifications: "ServerNotification" };
  for (const [kind, name] of Object.entries(unions)) {
    defs[name] = {
      oneOf: Object.entries(doc[kind]).map(([method, e]) => ({
        properties: { method: { enum: [method] }, ...(e.params ? { params: e.params } : {}) },
        required: ["method", ...(e.paramsRequired ? ["params"] : [])],
      })),
    };
  }
  return defs;
}

test("Hangar's real subset: unchanged against itself, and a change in a real type is seen", () => {
  const subset = readJson(path.join(HERE, "subset.json"));
  const doc = readJson(path.join(HERE, "schema.subset.json"));
  const before = defsFromReduced(doc);
  assert.deepEqual(diffSchemas({ pinned: before, latest: structuredClone(before), subset }).changes, []);

  const after = structuredClone(before);
  after.ThreadStartParams.properties.aNewField = { type: "string" };
  after.AskForApproval.oneOf[0].enum.push("always");
  delete after.ThreadItem.oneOf[0];
  const rowsOf = diffSchemas({ pinned: before, latest: after, subset }).changes.map((c) => [c.scope, c.path, c.kind]);
  assert.deepEqual(rowsOf.filter(([scope]) => scope === "type ThreadStartParams"), [["type ThreadStartParams", ".aNewField", "field-added"]]);
  assert.deepEqual(rowsOf.filter(([scope]) => scope === "type AskForApproval"), [["type AskForApproval", "", "enum-added"]]);
  assert.ok(rowsOf.some(([scope, , kind]) => scope === "type ThreadItem" && kind === "variant-removed"));
});

// --------------------------------------------------------------------------------------- the script

/** Two schema folders as `generate-json-schema --out` and the workflow leave them. */
function folders(pinnedBundle, latestBundle, { pinnedVersion = "codex-cli 1.0.0", latestVersion = "codex-cli 1.1.0" } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "codex-schema-cli-"));
  const write = (name, b, version) => {
    const dir = path.join(root, name);
    fs.mkdirSync(dir);
    if (b) fs.writeFileSync(path.join(dir, "codex_app_server_protocol.schemas.json"), JSON.stringify(b));
    if (version) fs.writeFileSync(path.join(dir, "codex-version.txt"), `${version}\n`);
    return dir;
  };
  const subset = path.join(root, "subset.json");
  fs.writeFileSync(subset, JSON.stringify(SUBSET));
  return { root, subset, pinned: write("pinned", pinnedBundle, pinnedVersion), latest: write("latest", latestBundle, latestVersion) };
}

function run(f) {
  const r = spawnSync(process.execPath, [SCRIPT, f.pinned, f.latest, "--subset", f.subset], { encoding: "utf8" });
  fs.rmSync(f.root, { recursive: true, force: true });
  return r;
}

test("exit 0 when the subset is unchanged", () => {
  const r = run(folders(pinned(), pinned(), { latestVersion: "codex-cli 1.0.1" }));
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /latest {2}codex-cli 1\.0\.1/);
  assert.match(r.stdout, /The subset is unchanged\./);
});

test("exit 1 and the diff on stdout when it changed", () => {
  const r = run(folders(pinned(), latest()));
  assert.equal(r.status, 1, r.stderr);
  assert.match(r.stdout, /clientRequests thing\/drop\n {2}- method removed/);
  assert.match(r.stdout, /10 changes in the subset\./);
});

test("exit 1 when the latest release no longer writes its bundle where it was", () => {
  const r = run(folders(pinned(), null));
  assert.equal(r.status, 1, r.stderr);
  assert.match(r.stdout, /latest schema\n {2}! cannot be reduced to the subset: .*codex_app_server_protocol\.schemas\.json is missing/);
});

test("exit 2, and nothing on stdout, when the diff cannot be made", () => {
  const wrongPin = run(folders(pinned(), latest(), { pinnedVersion: "codex-cli 0.9.0" }));
  assert.equal(wrongPin.status, 2);
  assert.match(wrongPin.stderr, /comes from "codex-cli 0\.9\.0", subset\.json pins codex-cli 1\.0\.0/);
  assert.equal(wrongPin.stdout, "");
  const noVersion = run(folders(pinned(), latest(), { latestVersion: null }));
  assert.equal(noVersion.status, 2);
  assert.match(noVersion.stderr, /codex-version\.txt is missing/);
  const noArgs = spawnSync(process.execPath, [SCRIPT], { encoding: "utf8" });
  assert.equal(noArgs.status, 2);
  assert.match(noArgs.stderr, /usage:/);
});
