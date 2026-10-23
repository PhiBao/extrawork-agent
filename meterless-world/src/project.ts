import { WorldModel, externalId } from "../vendor/reference/src/index.ts";

export const METERLESS_COMMIT = "1742dec56a66e58eef9dda2d2b865823758ca8cd";

export type Ref = { type: string; id: string };

export type Projection = {
  caseId: string;
  observedAt: string;
  revision: number;
  entities: Array<{ type: string; id: string; name?: string; attrs?: Record<string, unknown> }>;
  links: Array<{ from: Ref; to: Ref; type: string }>;
  facts: Array<{ about: Ref; predicate: string; value: unknown; confidence?: number }>;
};

export type ApplyResult = {
  commit: string;
  eventCount: number;
  auditCount: number;
  shape: string;
  unchangedOnReplay: boolean;
  openEvidenceLinks: number;
};

type World = InstanceType<typeof WorldModel>;

const source = (at: string, runId: string) => ({
  kind: "api" as const,
  by: "extrawork",
  at,
  runId,
});

export function createWorld(storage: "memory" | "local", directory?: string): World {
  return new WorldModel({
    storage,
    namespace: "extrawork",
    directory,
    // Similarities never reach these bands. Financial identities use external keys anyway.
    resolver: { autoMergeAt: 1.01, reviewBand: [1.01, 1.02] },
    conflictPolicy: "manual",
  });
}

export async function applyProjection(world: World, body: Projection): Promise<ApplyResult> {
  if (!body.caseId || !body.observedAt) {
    throw new Error("projection requires caseId and observedAt");
  }
  const runId = `${body.caseId}:r${body.revision}`;
  await writeProjection(world, body, runId);
  const before = world.stateShapeCanonical();
  await writeProjection(world, body, runId);
  const after = world.stateShapeCanonical();
  const findingId = body.entities.find((e) => e.type === "finding")?.id;
  let openEvidenceLinks = 0;
  if (findingId) {
    openEvidenceLinks = world.view("graph").neighborsOf(externalId("finding", "extrawork", findingId)).length;
  }
  return {
    commit: METERLESS_COMMIT,
    eventCount: world.events().length,
    auditCount: world.audit().length,
    shape: after,
    unchangedOnReplay: before === after,
    openEvidenceLinks,
  };
}

async function writeProjection(world: World, body: Projection, runId: string): Promise<void> {
  const prov = source(body.observedAt, runId);
  for (const entity of body.entities) {
    const id = externalId(entity.type, "extrawork", entity.id);
    const existing = world.view("graph").entity(id);
    if (existing) continue;
    await world.upsertEntity({
      type: entity.type,
      externalKey: { system: "extrawork", id: entity.id },
      name: entity.name,
      attrs: { ...(entity.attrs ?? {}), caseId: body.caseId },
      source: prov,
    });
  }
  for (const link of body.links) {
    // Unique relationship type per target so multiple evidence links do not supersede each other.
    const relType = `${link.type}:${link.to.type}:${link.to.id}`;
    await world.relate({
      from: externalId(link.from.type, "extrawork", link.from.id),
      type: relType,
      to: externalId(link.to.type, "extrawork", link.to.id),
      validFrom: body.observedAt,
      source: prov,
    });
  }
  for (const fact of body.facts) {
    await world.assertFact({
      about: externalId(fact.about.type, "extrawork", fact.about.id),
      predicate: fact.predicate,
      value: fact.value,
      confidence: fact.confidence ?? 1,
      source: prov,
    });
  }
}

export function caseView(world: World, caseId: string) {
  const caseEntity = externalId("case", "extrawork", caseId);
  const graph = world.view("graph");
  const entity = graph.entity(caseEntity);
  return {
    commit: METERLESS_COMMIT,
    found: Boolean(entity),
    eventCount: world.events().length,
    auditCount: world.audit().length,
    neighbors: entity ? graph.neighborsOf(caseEntity).map((item) => ({ id: item.id, type: item.type, name: item.name ?? null })) : [],
    shape: world.stateShapeCanonical(),
  };
}
