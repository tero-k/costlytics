/**
 * Typed client for the Costlytics HTTP API (`crates/api/src/handlers.rs` /
 * `crates/api/src/routes.rs`).
 *
 * Types here mirror the *wire* JSON shape actually produced by the live
 * Axum handlers, verified by reading `crates/api/src/handlers.rs` and
 * `crates/domain/src/{cost,filters,dimensions}.rs` and by making real
 * requests against a running `cargo run -p api` backed by the generated
 * fixtures. In a couple of places the handler's *response* struct
 * (`TimeseriesResponse`/`BreakdownResponse`/`CompareResponse` in
 * `handlers.rs`) adds fields on top of the plainer `domain::cost::*Result`
 * types the query layer returns internally — the interfaces below name
 * themselves after the query-layer types (`TimeSeriesResult`,
 * `BreakdownResult`, `CompareResult`) per this task's required function
 * signatures, but their *shape* matches the richer wire response, not the
 * narrower internal type of the same family name.
 */

// ---------------------------------------------------------------------------
// Shared enums / value types
// ---------------------------------------------------------------------------

export type CostMetric = 'amortized' | 'billed' | 'list' | 'contracted';

export type TimeGranularity = 'day' | 'month' | 'year';

export type Dimension =
  | 'service'
  | 'account'
  | 'region'
  | 'availability_zone'
  | 'charge_category'
  | 'pricing_category'
  | 'resource';

export type TagOperator = 'eq' | 'ne' | 'exists' | 'not_exists' | 'in';

export interface TagFilter {
  key: string;
  operator: TagOperator;
  /** Values for eq/ne/in; empty for exists/not_exists. */
  values: string[];
}

/**
 * The six predicate fields accepted on every `/cost/*` request
 * (`FilterFields` in `handlers.rs`, flattened into the request body).
 * All fields are optional on the wire (server defaults missing ones to
 * `[]`) but are always present on *responses* that echo filters back
 * (e.g. `CompareResponse.current_filters`), so this type keeps them
 * required and callers use `{}` cast or spread partials when building a
 * request (see `emptyFilterFields()` below).
 */
export interface FilterFields {
  accounts: string[];
  services: string[];
  regions: string[];
  charge_categories: string[];
  resource_ids: string[];
  tags: TagFilter[];
}

/** Convenience default for building request bodies. */
export function emptyFilterFields(): FilterFields {
  return {
    accounts: [],
    services: [],
    regions: [],
    charge_categories: [],
    resource_ids: [],
    tags: [],
  };
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/summary
// ---------------------------------------------------------------------------

export interface SummaryRequest extends Partial<FilterFields> {
  source_id?: string;
  /** `YYYY-MM-DD`. */
  start: string;
  /** `YYYY-MM-DD`, exclusive. */
  end: string;
  metric?: CostMetric;
}

export interface CostSummary {
  metric: CostMetric;
  currency: string;
  total: number;
  row_count: number;
  source_format: string | null;
  query_ms: number;
  /** ISO-8601 UTC datetime. */
  start: string;
  /** ISO-8601 UTC datetime. */
  end: string;
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/timeseries
// ---------------------------------------------------------------------------

export interface TimeseriesRequest extends Partial<FilterFields> {
  source_id?: string;
  start: string;
  end: string;
  metric?: CostMetric;
  granularity?: TimeGranularity;
  group_by?: Dimension;
}

export interface TimeSeriesPoint {
  /** ISO-8601 UTC datetime, truncated to the requested granularity. */
  period: string;
  group: string | null;
  total: number;
  row_count: number;
}

/**
 * Wire shape of `POST /api/v1/cost/timeseries`'s response
 * (`TimeseriesResponse` in `handlers.rs`).
 */
export interface TimeSeriesResult {
  metric: CostMetric;
  currency: string;
  granularity: TimeGranularity;
  series: TimeSeriesPoint[];
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/breakdown
// ---------------------------------------------------------------------------

export interface BreakdownRequest extends Partial<FilterFields> {
  source_id?: string;
  start: string;
  end: string;
  metric?: CostMetric;
  dimension: Dimension;
  limit?: number;
}

export interface BreakdownRow {
  key: string | null;
  total: number;
  row_count: number;
}

/**
 * Wire shape of `POST /api/v1/cost/breakdown`'s response
 * (`BreakdownResponse` in `handlers.rs`).
 */
export interface BreakdownResult {
  metric: CostMetric;
  currency: string;
  dimension: Dimension;
  rows: BreakdownRow[];
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/compare
// ---------------------------------------------------------------------------

export interface CompareRequest {
  source_id?: string;
  current_start: string;
  current_end: string;
  previous_start: string;
  previous_end: string;
  metric?: CostMetric;
  dimension?: Dimension;
  /** Predicate fields for the current period only; defaults to no filter. */
  current?: Partial<FilterFields>;
  /** Predicate fields for the previous period only; defaults to no filter. */
  previous?: Partial<FilterFields>;
}

export interface CompareRow {
  key: string | null;
  current: number;
  previous: number;
  absolute_change: number;
  /** `null` when `previous === 0` (percentage change is undefined). */
  percentage_change: number | null;
}

/**
 * Wire shape of `POST /api/v1/cost/compare`'s response (`CompareResponse`
 * in `handlers.rs`).
 */
export interface CompareResult {
  metric: CostMetric;
  currency: string;
  dimension: Dimension | null;
  rows: CompareRow[];
  current_filters: FilterFields;
  previous_filters: FilterFields;
}

// ---------------------------------------------------------------------------
// GET /api/v1/filter-values/*
// ---------------------------------------------------------------------------

export type FilterValuesDimension = 'services' | 'accounts' | 'regions' | 'tag-keys';

interface FilterValuesResponse {
  values: string[];
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

interface ErrorResponseBody {
  error: string;
}

/** Thrown by every client function below on a non-2xx HTTP response. */
export class ApiError extends Error {
  readonly status: number;
  readonly body: ErrorResponseBody;

  constructor(status: number, body: ErrorResponseBody) {
    super(body.error);
    this.name = 'ApiError';
    this.status = status;
    this.body = body;
  }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

async function postJson<TResponse>(path: string, body: unknown): Promise<TResponse> {
  const res = await fetch(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  return parseJsonOrThrow<TResponse>(res);
}

async function getJson<TResponse>(path: string): Promise<TResponse> {
  const res = await fetch(path, {
    method: 'GET',
    headers: { 'content-type': 'application/json' },
  });
  return parseJsonOrThrow<TResponse>(res);
}

async function parseJsonOrThrow<TResponse>(res: Response): Promise<TResponse> {
  const data: unknown = await res.json();
  if (!res.ok) {
    throw new ApiError(res.status, data as ErrorResponseBody);
  }
  return data as TResponse;
}

export async function getSummary(req: SummaryRequest): Promise<CostSummary> {
  return postJson<CostSummary>('/api/v1/cost/summary', req);
}

export async function getTimeseries(req: TimeseriesRequest): Promise<TimeSeriesResult> {
  return postJson<TimeSeriesResult>('/api/v1/cost/timeseries', req);
}

export async function getBreakdown(req: BreakdownRequest): Promise<BreakdownResult> {
  return postJson<BreakdownResult>('/api/v1/cost/breakdown', req);
}

export async function getCompare(req: CompareRequest): Promise<CompareResult> {
  return postJson<CompareResult>('/api/v1/cost/compare', req);
}

export async function getFilterValues(dimension: FilterValuesDimension): Promise<string[]> {
  const { values } = await getJson<FilterValuesResponse>(`/api/v1/filter-values/${dimension}`);
  return values;
}
