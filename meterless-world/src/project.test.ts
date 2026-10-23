import assert from "node:assert/strict";
import { applyProjection, createWorld } from "./project.ts";

const observedAt = "2026-10-18T15:04:00.000Z";
const projection = {
  caseId: "harbor-annex",
  observedAt,
  revision: 1,
  entities: [
    { type: "case", id: "harbor-annex", name: "COR-184", attrs: { request: "COR-184" } },
    { type: "finding", id: "dup-lift", name: "Lift billed twice", attrs: { state: "supported" } },
    { type: "document", id: "invoice-902", name: "INV-902", attrs: { kind: "invoice" } },
    { type: "document", id: "cost-export", name: "cost-export.csv", attrs: { kind: "cost_export" } },
    { type: "cost_line", id: "L3", name: "Scissor lift Friday", attrs: { amountCents: 100000 } },
    { type: "cost_line", id: "L4", name: "Scissor lift Friday", attrs: { amountCents: 100000 } },
  ],
  links: [
    { from: { type: "case", id: "harbor-annex" }, to: { type: "finding", id: "dup-lift" }, type: "has" },
    { from: { type: "finding", id: "dup-lift" }, to: { type: "document", id: "invoice-902" }, type: "cites" },
    { from: { type: "finding", id: "dup-lift" }, to: { type: "document", id: "cost-export" }, type: "cites" },
    { from: { type: "finding", id: "dup-lift" }, to: { type: "cost_line", id: "L4" }, type: "cites" },
  ],
  facts: [
    { about: { type: "finding", id: "dup-lift" }, predicate: "evidenceState", value: "supported" },
    { about: { type: "cost_line", id: "L3" }, predicate: "amountCents", value: 100000 },
    { about: { type: "cost_line", id: "L4" }, predicate: "amountCents", value: 100000 },
  ],
};

const world = createWorld("memory");
const first = await applyProjection(world, projection);
assert.equal(first.unchangedOnReplay, true, "replaying the same projection must not change canonical state");
assert.ok(first.eventCount > 0, "the working feature must write world events");
assert.ok(first.auditCount > 0, "every mutation must leave an audit entry");
assert.ok(first.openEvidenceLinks >= 3, `multiple evidence links must stay open, got ${first.openEvidenceLinks}`);

const similar = createWorld("memory");
await similar.upsertEntity({
  type: "cost_line",
  externalKey: { system: "extrawork", id: "L3" },
  name: "Scissor lift Friday",
  attrs: { amountCents: 100000 },
  source: { kind: "api", at: observedAt, by: "extrawork" },
});
await similar.upsertEntity({
  type: "cost_line",
  externalKey: { system: "extrawork", id: "L4" },
  name: "Scissor lift Friday",
  attrs: { amountCents: 100000 },
  source: { kind: "api", at: observedAt, by: "extrawork" },
});
const lines = similar.view("graph").query({ type: "cost_line" });
assert.equal(lines.length, 2, "similar cost-line names must not fuzzy-merge when external keys differ");
assert.equal(similar.mergeQueue().length, 0, "financial identities must not enter the merge queue");

console.log(`extrawork world-model feature: events=${first.eventCount} audit=${first.auditCount} links=${first.openEvidenceLinks} replay=unchanged`);
