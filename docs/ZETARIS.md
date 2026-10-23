# Zetaris connection status and options

## What is verified

- **Local Zetaris 2.4.3** is running from the licensed bundle. `verify.sh` passes **9/9**: postgres,
  driver, API, UI, init, bootstrap, UI config, admin login, and a query through UI → API → driver →
  Spark.
- ExtraWork's Gateway client runs a live `SELECT 1` plus the case joins: **4 rows for Harbor Annex**
  and **5 rows for Northgate Plant**. The proof records `joinMode: zetaris_live`, the extract count,
  and `queryClosed: true`.
- The two case tables are registered from a local PostgreSQL datasource (`harbor_pg`) and hold both
  cases.
- The AWS quickstart stack `zetaris-main-9acdd8a3` is `CREATE_COMPLETE` in `ap-southeast-1`.
- The registration callback returned HTTP 200.
- The EKS cluster `zetaris-extrawork-8b3246e1` reached **ACTIVE** with two running nodes, then was
  deleted during the AWS cleanup. Zetaris had created the Postgres stateful set and Spark operator.
- The AWS application phase stopped when the Load Balancer Controller tried to create a load balancer.
- All billable AWS resources from the Zetaris stack were removed: EKS, nodes, EFS, EBS, NAT, EIP,
  VPC, S3, IAM roles, log group. No running Zetaris AWS resources remain.

## The exact AWS block

CloudTrail records repeated `CreateLoadBalancer` failures from the Zetaris controller:

```
OperationNotPermittedException
This AWS account currently does not support creating load balancers.
For more information, please contact AWS Support.
```

Request IDs: `2d147cec-9e02-4441-8557-aab8b40f5f4b`, `d03c0549-b85a-4b50-ae80-1668b6d1315f`,
`2af88fab-03a7-45a7-bdec-76e1d72f2659`.

Target groups are created successfully. The block is **not**:

- an IAM policy or SCP (the account is not in an organization);
- a Zetaris permission problem (the CloudTrail error is returned by the ELB service);
- regional (root also fails in `us-east-1`);
- a load-balancer quota (the limit API returns normal values).

It is an account-level AWS restriction. Only AWS Support can lift it.

## Options

### 1. Local Zetaris (installed and verified)

The licensed bundle runs on this machine at `~/zetaris-local/zetaris-platform`.

- UI/Gateway: `http://127.0.0.1:3010` (port 3000 was already used by another app).
- API: `http://127.0.0.1:8888`; JDBC: `127.0.0.1:10000`.
- Admin: `admin@extrawork.dev`; password in `~/zetaris-local/zetaris-platform/.admin-password`.
- Organisation id: `1`.
- Vendor `verify.sh`: **9/9 passed**, including login and a query through UI → API → driver → Spark.
- The app's live join uses `harbor_pg.project_lines` and `harbor_pg.evidence_extracts`, registered
  from the local PostgreSQL datasource. Harbor returns four rows and Northgate five; both queries are
  closed after reading.

The bundle's `.env` points at `zetarishackathon.azurecr.io`. Signing in there with the Zetaris Cloud
account name failed because the registry requires the separate token issued on the download page. The
images are also published under `zetregistry.azurecr.io`, and the running AWS cluster's image-pull
secret authenticates there. This install uses that mirror:

```bash
ZETARIS_IMAGE_REGISTRY=zetregistry.azurecr.io ./scripts/setup-local-zetaris.sh
```

The setup script rewrites the three image references and the copied `preflight.sh` registry check to
the mirror. The supplied `.env`, admin password, and image pull credentials are not committed.

### 2. AWS Support

The Support API requires Premium support, so a case cannot be opened from this environment. In the
AWS Console, open **Support → Create case** and use the text in `docs/AWS-ELB-CASE.md`. Basic support
may accept an account-level case; a technical case may require Developer support.

After ELB is enabled, redeploy the application phase from the cluster's **Lifecycle** tab in Zetaris
Cloud. The cluster is preserved, so the control plane and nodes do not need to be rebuilt.

### 3. Another AWS account

Zetaris Cloud can register a second AWS account. The cluster currently costs roughly **$8/day**
(EKS control plane, two nodes, NAT, storage). Delete it if the wait is likely to be long; Zetaris
retains EFS and S3 separately.

## First live query

Once a Gateway is running:

```bash
# health through the Gateway proxy
curl -s http://127.0.0.1:3000/api/proxy/lightning-database/databases \
  -H "Authorization: Bearer $ZETARIS_API_KEY" \
  -H "X-Request-ID: $(uuidgen)"
```

ExtraWork's client supports the Gateway proxy, the documented REST API, and the SaaS API. For the
case join, the instance must expose `project_lines` and `evidence_extracts` (a single namespace is
allowed, for example `harbor.project_lines`). The tables can be registered from the app's PostgreSQL
and the evidence CSV using Zetaris's datasource model; they are not copied into this repository.

Until then, ExtraWork performs the same join locally across PostgreSQL and the separate evidence CSV,
and the proof panel says so.
