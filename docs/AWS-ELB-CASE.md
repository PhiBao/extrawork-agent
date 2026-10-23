# AWS Support case text

The Support API refused case creation with `SubscriptionRequiredException` (Premium support is
required for the API). Open a case in the AWS Console instead:

**Support → Create case → Account and billing** (or Technical, if available), region
`ap-southeast-1`.

Subject:

```
Enable Elastic Load Balancing for account 381492277789 (ap-southeast-1)
```

Body:

```
Account: 381492277789
Region: ap-southeast-1 (Singapore)
Cluster: zetaris-extrawork-8b3246e1 (EKS, ACTIVE)

Creating an EKS cluster succeeded, and target groups are created successfully, but every attempt to
create a load balancer fails with:

  OperationNotPermittedException: This AWS account currently does not support creating load
  balancers. For more information, please contact AWS Support.

CloudTrail request IDs (elasticloadbalancing.amazonaws.com, CreateLoadBalancer):
- 2d147cec-9e02-4441-8557-aab8b40f5f4b
- d03c0549-b85a-4b50-ae80-1668b6d1315f
- 2af88fab-03a7-45a7-bdec-76e1d72f2659

The account is ACTIVE on the PAID plan and has available credits. We need Elastic Load Balancing
enabled for this account in ap-southeast-1 so the EKS application can finish deploying its services.

Please enable load balancer creation for this account/region, or tell us what verification is
required.
```

Do not include access keys, passwords, or the root email in the case.
