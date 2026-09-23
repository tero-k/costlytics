import { describe, expect, it } from 'vitest';
import { formToSource, slugify, sourceToForm, type SourceFormValues } from '../sourceForm.ts';

const base: SourceFormValues = {
  id: '',
  name: 'Prod Exports',
  kind: 's3',
  location: 'my-bucket/exports/data',
  region: 'eu-north-1',
  format: 'auto',
  authType: 'credential_chain',
  profile: '',
  keyId: '',
};

describe('slugify', () => {
  it('lowercases and dashes', () => {
    expect(slugify('  Prod Exports (EU)! ')).toBe('prod-exports-eu');
  });
});

describe('formToSource', () => {
  it('prefixes s3:// and slugifies an empty id', () => {
    const s = formToSource(base);
    expect(s.id).toBe('prod-exports');
    expect(s.s3_uri).toBe('s3://my-bucket/exports/data');
    expect(s.aws_region).toBe('eu-north-1');
    expect(s.aws_profile).toBeNull();
    expect(s.auth).toEqual({ type: 'credential_chain' });
  });

  it('keeps the profile for credential chain and the key id for access keys', () => {
    expect(formToSource({ ...base, profile: 'sso-prod' }).aws_profile).toBe('sso-prod');
    const s = formToSource({ ...base, authType: 'access_key', keyId: ' AKIA1 ', profile: 'ignored' });
    expect(s.auth).toEqual({ type: 'access_key', key_id: 'AKIA1' });
    expect(s.aws_profile).toBeNull();
  });

  it('drops all S3 fields for local folders', () => {
    const s = formToSource({ ...base, kind: 'local', location: 'C:\\data\\focus', authType: 'access_key', keyId: 'x' });
    expect(s.s3_uri).toBe('C:\\data\\focus');
    expect(s.aws_region).toBeNull();
    expect(s.auth).toEqual({ type: 'credential_chain' });
  });
});

describe('sourceToForm', () => {
  it('round-trips an access-key S3 source', () => {
    const src = formToSource({ ...base, id: 'p', authType: 'access_key', keyId: 'AKIA1' });
    const form = sourceToForm({ ...src, has_secret: true });
    expect(form).toMatchObject({ id: 'p', kind: 's3', location: 's3://my-bucket/exports/data', authType: 'access_key', keyId: 'AKIA1' });
    expect(formToSource(form)).toEqual(src);
  });
});
