# Typed host services

The engine passes a `RuleContext` containing the immutable project view and a type-indexed `ServiceRegistry` to trusted capabilities. Applications register semantic or computational services explicitly; duplicate concrete service types are rejected instead of silently replacing an implementation.

Adapter crates are peers:

- OpenBIM implementations can provide semantic/property/relationship services.
- Axiolid implementations can provide geometry services for IFC, proprietary CAD, or any source able to lower geometry into Axiolid.
- ICDD implementations can provide project assembly and cross-document link services.
- Alternate geometry backends register the same source-neutral service interfaces.

Rule packages cannot register services and cannot supply executable code. Missing required services must produce an explicit not-evaluated/backend-unavailable outcome, never a pass.
