# Concepts

A **Monitor** names a target, a kind, and a check interval. Kinds include HTTP, TCP, ICMP, Kubernetes resources, and checks supplied by a host agent. `monitra monitor add --name example --target https://example.com --kind http --interval 30` creates one.

A **Check result** records when a check ran, whether it succeeded, latency, and an optional message. `monitra monitor history <ID>` reads those observations.

An **Alert** records a monitor status transition and its notification outcome. `monitra alert list` shows recorded events, including whether delivery was sent or queued for retry.

An **Agent** is a registered process that pushes local checks to the backend. A **Region** is an optional label on an agent's network vantage point. Region-tagged agents can probe HTTP, TCP, and ICMP targets for comparison through `monitra monitor regions`; Kubernetes checks are not assigned for regional probing.

Statuses include `Pending` before the first check, `Up`, `Down`, `Paused`, `Stale` when a feed or check becomes unreliable, and `Unknown` when an invalid stored interval quarantines that monitor. A quarantined monitor is skipped and has reason `invalid stored interval`; it does not fire a down alert. Monitra treats missing evidence as unknown or stale, rather than claiming the target is down. See [Agents](agents.md) and [Alerting](alerting.md).
