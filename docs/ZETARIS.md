# Zetaris connection status and options

## What is verified

- The AWS quickstart stack `zetaris-main-9acdd8a3` is `CREATE_COMPLETE` in `ap-southeast-1`.
- The registration callback returned HTTP 200.
- The EKS cluster `zetaris-extrawork-8b3246e1` is **ACTIVE**, with two running `m7i-flex.large` nodes.
- Zetaris created the Postgres stateful set and the Spark operator.
- The application phase stopped when the AWS Load Balancer Controller tried to create a load balancer.

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

### 1. Local Zetaris (fastest, real platform)

The licensed bundle runs on this machine (16 GB RAM, 16 CPUs, 774 GB free).

1. Sign in at <https://cloud.enterprise.zetaris.com/dashboard/download> with the account used to
   create the AWS cluster.
2. Download `zetaris-platform.zip` and keep the registry sign-in command shown there.
3. Put the zip at `vendor/zetaris-platform.zip` and run the registry login.
4. Run:

```bash
./scripts/setup-local-zetaris.sh
```

The script configures the supplied `.env`, generates an admin password in
`~/zetaris-local/.admin-password`, starts Compose, waits for the Gateway, and writes the Gateway
connection to `.env`.

The bundle and registry access are licensed to the Zetaris account holder and are not included in
this repository.

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
