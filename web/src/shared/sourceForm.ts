import type { ConfiguredSourceType, DataSourceSettings, SourceSettings } from '../api.ts';

/** Raw values of the Settings page's add/edit form. */
export interface SourceFormValues {
  id: string;
  name: string;
  kind: 's3' | 'local';
  /** S3 URI (with or without `s3://`) or local folder path. */
  location: string;
  region: string;
  format: ConfiguredSourceType;
  authType: 'credential_chain' | 'access_key';
  profile: string;
  keyId: string;
}

export function slugify(name: string): string {
  return name
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 48);
}

function blankToNull(value: string): string | null {
  const t = value.trim();
  return t === '' ? null : t;
}

export function formToSource(v: SourceFormValues): DataSourceSettings {
  const s3 = v.kind === 's3';
  let location = v.location.trim();
  if (s3 && !location.startsWith('s3://')) location = `s3://${location}`;
  const accessKey = s3 && v.authType === 'access_key';
  return {
    id: v.id.trim() || slugify(v.name),
    name: v.name.trim(),
    s3_uri: location,
    source_type: v.format,
    aws_region: s3 ? blankToNull(v.region) : null,
    aws_profile: s3 && !accessKey ? blankToNull(v.profile) : null,
    role_arn: null,
    auth: accessKey ? { type: 'access_key', key_id: v.keyId.trim() } : { type: 'credential_chain' },
  };
}

export function sourceToForm(s: SourceSettings): SourceFormValues {
  return {
    id: s.id,
    name: s.name,
    kind: s.s3_uri.startsWith('s3://') ? 's3' : 'local',
    location: s.s3_uri,
    region: s.aws_region ?? '',
    format: s.source_type,
    authType: s.auth.type,
    profile: s.aws_profile ?? '',
    keyId: s.auth.type === 'access_key' ? s.auth.key_id : '',
  };
}
