import fixtures from "../../../fixtures/stage5-http-contracts.json";
import type {
  AnalyticsSnapshot,
  GatewayMetrics,
  Model,
  ModelMapping,
  RequestLog,
  Upstream
} from "../types/api";

// These assignments make tsc reject drift between the checked-in HTTP fixtures
// and the frontend's API types without introducing a frontend test framework.
const upstream: Upstream = fixtures.after.upstream;
const model: Model = fixtures.after.model;
const modelMapping: ModelMapping = fixtures.after.model_mapping;
const requestLog: RequestLog = fixtures.after.request_log;
const analytics: AnalyticsSnapshot = fixtures.after.analytics;
const gatewayMetrics: GatewayMetrics = fixtures.after.gateway_metrics;

export const stage5ContractFixtures = {
  upstream,
  model,
  modelMapping,
  requestLog,
  analytics,
  gatewayMetrics
};
