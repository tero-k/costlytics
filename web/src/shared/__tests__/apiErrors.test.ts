import { describe, expect, it } from 'vitest';
import { ApiError, toApiError } from '../../api.ts';

describe('toApiError', () => {
  it('maps service error with conflict kind to 409', () => {
    const err = toApiError({ kind: 'conflict', message: 'm' });
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(409);
    expect(err.message).toBe('m');
  });

  it('maps service error with bad_request kind to 400', () => {
    const err = toApiError({ kind: 'bad_request', message: 'x' });
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(400);
    expect(err.message).toBe('x');
  });

  it('maps service error without kind to 500', () => {
    const err = toApiError({ message: 'y' });
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(500);
    expect(err.message).toBe('y');
  });

  it('maps plain string error to 400', () => {
    const err = toApiError('invalid args');
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(400);
    expect(err.message).toBe('invalid args');
  });
});
