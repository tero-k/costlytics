import { beforeEach, describe, expect, it } from 'vitest';
import { createDimensionPicker } from '../dimensionPicker.ts';

// Deliberately generic paramName/elementId (not "service"/"account") — the
// module is generic and these tests must not be coupled to any one page.
const PARAM = 'widget';
const ELEMENT_ID = 'widget-picker';

function setUrl(href: string): void {
  // jsdom updates `window.location` in place from `history.pushState`
  // without triggering a real navigation (unlike assigning
  // `window.location.href` directly, which jsdom does not support).
  window.history.pushState(null, '', href);
}

describe('createDimensionPicker', () => {
  beforeEach(() => {
    document.body.innerHTML = `<select id="${ELEMENT_ID}"></select>`;
    // Same-origin relative URL — jsdom's default origin (http://localhost/)
    // is used since `pushState` rejects cross-origin URLs.
    setUrl('/detail.html');
  });

  it('populateOptions populates the <select> with the given values', () => {
    const picker = createDimensionPicker({ paramName: PARAM, elementId: ELEMENT_ID });
    picker.populateOptions(['Alpha', 'Beta', 'Gamma']);

    const select = document.querySelector<HTMLSelectElement>(`#${ELEMENT_ID}`)!;
    const values = Array.from(select.options).map((o) => o.value);
    expect(values).toEqual(['Alpha', 'Beta', 'Gamma']);
  });

  it('selecting a value updates the URL query param via history.replaceState, without a page reload', () => {
    const picker = createDimensionPicker({ paramName: PARAM, elementId: ELEMENT_ID });
    picker.populateOptions(['Alpha', 'Beta', 'Gamma']);

    let onChangeValue: string | null | undefined;
    picker.init((value) => {
      onChangeValue = value;
    });

    const select = document.querySelector<HTMLSelectElement>(`#${ELEMENT_ID}`)!;
    select.value = 'Beta';
    select.dispatchEvent(new Event('change'));

    // URL was updated in place (no navigation happened — jsdom would not
    // have thrown, but we confirm the actual document/location is intact
    // and reflects the new query param).
    expect(new URL(window.location.href).searchParams.get(PARAM)).toBe('Beta');
    expect(picker.getUrlParam()).toBe('Beta');
    expect(onChangeValue).toBe('Beta');
  });

  it('loading with a valid value already in the URL query param pre-selects it', () => {
    setUrl(`/detail.html?${PARAM}=Beta`);

    const picker = createDimensionPicker({ paramName: PARAM, elementId: ELEMENT_ID });
    const initial = picker.populateOptions(['Alpha', 'Beta', 'Gamma']);

    expect(initial).toBe('Beta');
    expect(picker.getSelected()).toBe('Beta');
    expect(picker.getUrlParam()).toBe('Beta');
  });

  it('falls back to the first available value and corrects the URL when the query param value is not in the fetched options list (?widget=BOGUS scenario)', () => {
    // Pins the exact scenario Session 9's reviewer hand-verified across
    // multiple browser sessions: a stale/bookmarked/tampered URL names a
    // value that the freshly-fetched options list doesn't contain.
    setUrl(`/detail.html?${PARAM}=BOGUS`);

    const picker = createDimensionPicker({ paramName: PARAM, elementId: ELEMENT_ID });
    const initial = picker.populateOptions(['Alpha', 'Beta', 'Gamma']);

    // Falls back to the first value in the (already-sorted) list.
    expect(initial).toBe('Alpha');
    expect(picker.getSelected()).toBe('Alpha');

    // The URL is corrected to match, not left pointing at the bogus value.
    expect(picker.getUrlParam()).toBe('Alpha');
    expect(new URL(window.location.href).searchParams.get(PARAM)).toBe('Alpha');
  });

  it('handles an empty options list without throwing, leaving selection and URL param null', () => {
    const picker = createDimensionPicker({ paramName: PARAM, elementId: ELEMENT_ID });

    let initial: string | null = 'unset';
    expect(() => {
      initial = picker.populateOptions([]);
    }).not.toThrow();

    expect(initial).toBeNull();
    expect(picker.getSelected()).toBeNull();
    expect(picker.getUrlParam()).toBeNull();
  });

  it('does nothing (does not throw) when the target <select> element is missing', () => {
    document.body.innerHTML = '';
    const picker = createDimensionPicker({ paramName: PARAM, elementId: ELEMENT_ID });

    expect(() => picker.populateOptions(['Alpha', 'Beta'])).not.toThrow();
    expect(picker.populateOptions(['Alpha', 'Beta'])).toBeNull();
    expect(() => picker.init(() => {})).not.toThrow();
  });
});
