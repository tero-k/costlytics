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

import { invoke } from '@tauri-apps/api/core';
import { argsFromPath, commandName, isTauri, statusForKind } from './transport.ts';

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

/** Thrown by every client function below on a failed request (HTTP non-2xx, or a rejected Tauri command). */
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

// ---------------------------------------------------------------------------
// Active source (Session 14 Task 3)
// ---------------------------------------------------------------------------

/**
 * Module-level "currently selected source" singleton, set by the shared
 * source-picker control (`shared/sourcePicker.ts`) on page load and on every
 * change. This is the SINGLE injection point threading `source_id` into
 * every outgoing request: `postJson`/`getJson` below merge it in
 * automatically, so none of the many per-component fetching modules
 * (`kpiCards.ts`, `trendChart.ts`, `entityKpi.ts`, `costChangesTable.ts`,
 * etc.) need to read or pass `source_id` themselves. `null` means "no
 * active source yet / picker unavailable" — requests are sent WITHOUT a
 * `source_id`, which the backend's own `resolve_source(state, None)`
 * fallback (first configured source) handles.
 */
let activeSourceId: string | null = null;

export function setActiveSourceId(id: string | null): void {
  activeSourceId = id;
}

export function getActiveSourceId(): string | null {
  return activeSourceId;
}

/**
 * Merges the active source into a POST request body, UNLESS the caller
 * already set `source_id` explicitly — an intentional override always wins
 * over the injected default. (As of this task, no caller does this, but the
 * precedence is load-bearing for `getSources()`-driven future call sites.)
 */
function withActiveSourceId(body: unknown): unknown {
  if (activeSourceId === null) return body;
  const record = body as Record<string, unknown>;
  if (record.source_id !== undefined) return body;
  return { ...record, source_id: activeSourceId };
}

/** Same precedence rule as {@link withActiveSourceId}, applied to a GET URL's query string instead of a JSON body. */
function withActiveSourceIdQuery(path: string): string {
  if (activeSourceId === null) return path;
  const url = new URL(path, window.location.origin);
  if (url.searchParams.has('source_id')) return path;
  url.searchParams.set('source_id', activeSourceId);
  return `${url.pathname}${url.search}`;
}

/** Tauri command errors are `service::ServiceError` (`{kind, message}`); argument-deserialization errors are plain strings.
 * @internal exported for tests */
export function toApiError(err: unknown): ApiError {
  if (err && typeof err === 'object' && 'message' in err) {
    const e = err as { kind?: unknown; message: unknown };
    return new ApiError(statusForKind(e.kind), { error: String(e.message) });
  }
  return new ApiError(400, { error: String(err) });
}

async function invokeCommand<TResponse>(path: string, req: unknown): Promise<TResponse> {
  try {
    return await invoke<TResponse>(commandName(path), { req });
  } catch (err) {
    throw toApiError(err);
  }
}

async function postJson<TResponse>(path: string, body: unknown): Promise<TResponse> {
  const withSource = withActiveSourceId(body);
  if (isTauri()) return invokeCommand<TResponse>(path, withSource);
  const res = await fetch(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(withSource),
  });
  return parseJsonOrThrow<TResponse>(res);
}

async function getJson<TResponse>(path: string): Promise<TResponse> {
  const withSource = withActiveSourceIdQuery(path);
  if (isTauri()) return invokeCommand<TResponse>(withSource, argsFromPath(withSource));
  const res = await fetch(withSource, {
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

// ---------------------------------------------------------------------------
// Page-level filters (per-page Service/Account filter widgets)
// ---------------------------------------------------------------------------

/**
 * Supplies the current page's filter selection (`shared/pageFilters.ts`),
 * merged into every `/cost/*` request below — the same single choke point
 * `withActiveSourceId` uses for the source, so none of the fetching modules
 * need to know page filters exist.
 */
let pageFilterProvider: (() => Partial<FilterFields>) | null = null;

export function setPageFilterProvider(provider: (() => Partial<FilterFields>) | null): void {
  pageFilterProvider = provider;
}

const FILTER_KEYS = ['accounts', 'services', 'regions', 'charge_categories', 'resource_ids', 'tags'] as const;

/**
 * Fills each predicate field the request leaves empty from `extra`. A field
 * the caller already set (e.g. a detail page's own `services: [selected]`
 * entity filter) always wins, so a page filter can only narrow on
 * dimensions the page doesn't already pin.
 * @internal exported for tests
 */
export function mergeFilters<T extends Partial<FilterFields>>(req: T, extra: Partial<FilterFields>): T {
  const merged: Partial<FilterFields> = { ...req };
  for (const key of FILTER_KEYS) {
    const value = extra[key];
    const existing = req[key];
    if (value && value.length > 0 && !(existing && existing.length > 0)) {
      (merged as Record<string, unknown>)[key] = value;
    }
  }
  return merged as T;
}

function withPageFilters<T extends Partial<FilterFields>>(req: T): T {
  return pageFilterProvider ? mergeFilters(req, pageFilterProvider()) : req;
}

/** @internal exported for tests */
export function withPageFiltersCompare(req: CompareRequest): CompareRequest {
  if (!pageFilterProvider) return req;
  const extra = pageFilterProvider();
  return { ...req, current: mergeFilters(req.current ?? {}, extra), previous: mergeFilters(req.previous ?? {}, extra) };
}

export async function getSummary(req: SummaryRequest): Promise<CostSummary> {
  return postJson<CostSummary>('/api/v1/cost/summary', withPageFilters(req));
}

export async function getTimeseries(req: TimeseriesRequest): Promise<TimeSeriesResult> {
  return postJson<TimeSeriesResult>('/api/v1/cost/timeseries', withPageFilters(req));
}

export async function getBreakdown(req: BreakdownRequest): Promise<BreakdownResult> {
  return postJson<BreakdownResult>('/api/v1/cost/breakdown', withPageFilters(req));
}

export async function getCompare(req: CompareRequest): Promise<CompareResult> {
  return postJson<CompareResult>('/api/v1/cost/compare', withPageFiltersCompare(req));
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/resource-search
// ---------------------------------------------------------------------------

export interface ResourceSearchRequest extends Partial<FilterFields> {
  source_id?: string;
  /** `YYYY-MM-DD`. */
  start: string;
  /** `YYYY-MM-DD`, exclusive. */
  end: string;
  metric?: CostMetric;
  /** Case-insensitive substring of the resource ID; must be non-blank. */
  q: string;
  /** Defaults to 50 server-side; clamped to 1..=200. */
  limit?: number;
}

/** Resource IDs matching `q` within the range/filters, most expensive first. */
export async function searchResources(req: ResourceSearchRequest): Promise<string[]> {
  const { values } = await postJson<FilterValuesResponse>('/api/v1/cost/resource-search', withPageFilters(req));
  return values;
}

/** A half-open `[start, end)` range (exclusive `end`, like every `/cost/*` request). */
export interface EstimateRange {
  start: string;
  end: string;
}

export type CostTier = 'none' | 'soft' | 'hard';

/** Mirrors `service::cost::EstimateResponse`. */
export interface EstimateResponse {
  remote: boolean;
  known: boolean;
  bytes: number;
  requests: number;
  usd: number;
  tier: CostTier;
  stats_missing: boolean;
  soft_limit_usd: number;
  hard_limit_usd: number;
}

/**
 * Estimates what a page's queries would read from S3: `scans` has one entry
 * per query, each listing the ranges that query reads. `sourceId` overrides
 * the active source (used while a source switch is still pending).
 */
export async function estimateCost(scans: EstimateRange[][], sourceId?: string): Promise<EstimateResponse> {
  const body = sourceId === undefined ? { scans } : { scans, source_id: sourceId };
  return postJson<EstimateResponse>('/api/v1/cost/estimate', body);
}

export async function getFilterValues(dimension: FilterValuesDimension): Promise<string[]> {
  const { values } = await getJson<FilterValuesResponse>(`/api/v1/filter-values/${dimension}`);
  return values;
}

/**
 * Typed client for `GET /api/v1/filter-values/tag-values?key=X` (distinct
 * values observed for the given tag key). Separate from `getFilterValues`
 * since this endpoint takes a required `key` query parameter (`TagValuesQuery`
 * in `crates/api/src/handlers.rs`'s `filter_values_tag_values` handler,
 * verified by reading it directly) rather than being a flat
 * `FilterValuesDimension`, and is thus a two-step fetch (tag-keys, then
 * tag-values-for-a-key) unlike the single-step pickers `getFilterValues`
 * serves.
 */
export async function getTagValues(key: string): Promise<string[]> {
  const { values } = await getJson<FilterValuesResponse>(
    `/api/v1/filter-values/tag-values?key=${encodeURIComponent(key)}`,
  );
  return values;
}

// ---------------------------------------------------------------------------
// GET /api/v1/sources
// ---------------------------------------------------------------------------

/**
 * Mirrors `crates/data/src/config.rs`'s `SourceType` wire format
 * (`#[serde(rename_all = "snake_case")]`).
 */
export type ConfiguredSourceType = 'auto' | 'cur2' | 'focus10' | 'focus12';

/**
 * Wire shape of one entry in `GET /api/v1/sources`' `sources` array
 * (`SourceEntry`/`SourceStatusResponse` in `crates/api/src/handlers.rs`,
 * `#[serde(tag = "state", rename_all = "snake_case")]`-flattened, so a
 * single JSON object is either
 * `{id, name, configured_type, state: "registered", detected_format, file_count}`
 * or `{id, name, configured_type, state: "skipped", reason}`). Modeled here
 * as one interface with optional fields (rather than a discriminated union)
 * since every call site either filters on `state` or displays whichever
 * fields are present — see Task 4's diagnostics page for the latter.
 */
export interface SourceStatus {
  id: string;
  name: string;
  configured_type: ConfiguredSourceType;
  state: 'pending' | 'registered' | 'skipped';
  /** `pending`: registration still running (startup, save or reload). */
  /** Present only when `state === 'registered'`. */
  detected_format?: string;
  /** Present only when `state === 'registered'`. */
  file_count?: number;
  /** Present only when `state === 'skipped'`. */
  reason?: string;
}

export interface SourcesResponse {
  sources: SourceStatus[];
  /**
   * The `source_id` `resolve_source(state, None)` would pick when a request
   * omits `source_id` — the FIRST configured source, which is not
   * necessarily registered/queryable (see `handlers.rs`'s doc comment on
   * this field). Callers that need a SELECTABLE default should instead pick
   * the first entry with `state === 'registered'` — see
   * `shared/sourcePicker.ts`.
   */
  default_source_id: string | null;
}

export async function getSources(): Promise<SourcesResponse> {
  return getJson<SourcesResponse>('/api/v1/sources');
}

// ---------------------------------------------------------------------------
// Settings (`service::app` in crates/service/src/app.rs)
// ---------------------------------------------------------------------------

/** `data::config::S3AuthConfig` wire shape. */
export type SourceAuth = { type: 'credential_chain' } | { type: 'access_key'; key_id: string };

/** `data::config::DataSource` wire shape. `s3_uri` is an `s3://` URI or a local folder path. */
export interface DataSourceSettings {
  id: string;
  name: string;
  s3_uri: string;
  source_type: ConfiguredSourceType;
  aws_region?: string | null;
  aws_profile?: string | null;
  role_arn?: string | null;
  auth: SourceAuth;
}

export interface SourceSettings extends DataSourceSettings {
  /** An access-key secret is stored in the OS keychain (the secret itself is never sent). */
  has_secret: boolean;
}

/** Mirrors `data::config::CostGuardConfig` (limits are USD per page load). */
export interface CostGuardSettings {
  enabled: boolean;
  soft_limit_usd: number;
  hard_limit_usd: number;
  egress_usd_per_gb: number;
  get_usd_per_1000: number;
}

export interface SettingsResponse {
  sources: SourceSettings[];
  cost_guard: CostGuardSettings;
}

export interface TestSourceResponse {
  detected_format: string;
  file_count: number;
  billing_periods: string[];
}

export async function getSettings(): Promise<SettingsResponse> {
  return getJson<SettingsResponse>('/api/v1/settings');
}

/** `secret`: a new access-key secret, or `null` to keep the stored one. */
export async function saveSource(
  source: DataSourceSettings,
  secret: string | null,
  isNew: boolean,
): Promise<SourceStatus> {
  return postJson<SourceStatus>('/api/v1/settings/source-save', { source, secret, is_new: isNew });
}

export async function testSource(
  source: DataSourceSettings,
  secret: string | null,
): Promise<TestSourceResponse> {
  return postJson<TestSourceResponse>('/api/v1/settings/source-test', { source, secret });
}

export async function deleteSource(id: string): Promise<SourcesResponse> {
  return postJson<SourcesResponse>('/api/v1/settings/source-delete', { id });
}

export async function reloadSource(id: string): Promise<SourceStatus> {
  return postJson<SourceStatus>('/api/v1/settings/source-reload', { id });
}

export async function saveCostGuard(settings: CostGuardSettings): Promise<CostGuardSettings> {
  return postJson<CostGuardSettings>('/api/v1/settings/cost-guard-save', settings);
}
