# Current Architecture

This document describes the runtime architecture after the staged restructuring. It is intentionally separate from protocol, feature, and user-facing documentation.

## Composition

`src/lib.rs` is the application composition root. It loads typed configuration, initializes telemetry, opens and migrates SQLite, performs startup maintenance, creates shared application state, composes the panel and proxy routers, starts the optional health worker, and coordinates graceful shutdown.

The public HTTP inventory remains 46 paths and 55 method/path combinations. Axum middleware assigns a sanitized request ID, applies the JSON body limit and CORS policy, and records request spans. API handlers return API-owned DTOs rather than serializing persistence records directly.

## Dependency Direction

- `api` owns panel and administrator HTTP handlers and DTO conversion.
- `proxy` owns Codex request preparation, route planning, attempts, streaming, and settlement.
- `auth` depends on its narrow persistence port rather than on the storage facade.
- `routing` selects internal route candidates. Candidates containing decrypted upstream credentials are crate-private.
- `storage` is split by domain behind a compatibility facade and remains the SQLite transaction owner.
- `config` owns typed runtime descriptors and environment/database/default precedence.

The API and proxy routers are composed only in `lib.rs`; neither registers the other. Authentication does not depend on API error types, and proxy execution does not call API handlers.

## Request And Settlement Lifecycle

Each proxy request is authenticated, planned, admitted against user and API-key limits, and executed against ordered upstream candidates. Every real attempt produces one persistence record. Retry, timeout, status, JSON, and SSE behavior remain part of the proxy pipeline.

`FinalizationLifecycle` and `FinalizationTracker` are the single ownership system for asynchronous settlement work. They track attempt persistence, dropped admission settlement, stream finalization, and upstream health writes. SSE completion, EOF, transport error, client disconnect, and cancellation all transfer finalization to this tracker. Graceful shutdown stops request acceptance and the background health worker, then drains tracked work before returning.

## Data And Secrets

SQLite migrations remain the schema source of truth. Multi-step business mutations and their audit or settlement writes use one transaction owner. Limit reads validate persisted policy, event, rate-window, and inflight timestamps before using them; corrupt values produce a sanitized data-integrity response.

Upstream API keys are encrypted at rest. Decrypted credentials exist only in crate-private values and are consumed at exactly two internal boundaries: proxy attempts build the upstream authorization header from a crate-private route candidate, and upstream health probes build their authorization header from a function-local decrypted value. Decrypted credentials are never serialized, logged, or returned. Sensitive persistence records do not implement response serialization or debug formatting. Logs include identifiers needed for diagnosis but do not include credentials, request bodies, raw corrupt values, or upstream response bodies.

## Verification Surfaces

Backend behavior is covered by Rust unit and HTTP integration tests organized by auth, proxy, routing, limits, settings, analytics, and security. Shared deterministic fixtures provide an injectable clock and controllable upstreams without sleeps. Frontend contracts, component behavior, type checking, and production builds are verified from `frontend/`.
