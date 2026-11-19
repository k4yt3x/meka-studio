import { describe, expect, it } from 'vitest';
import { pdfPlace } from './pdf-place';

describe('a place in a PDF, as kept', () => {
  it('is taken where it is whole', () => {
    const place = { page: 3, zoom: 'page-width', left: 0, top: 612.5 };
    expect(pdfPlace(place)).toEqual(place);
    expect(pdfPlace({ ...place, zoom: 125 })).toEqual({ ...place, zoom: 125 });
  });

  it('is left out where a browser kept something else under the file', () => {
    expect(pdfPlace(undefined)).toBeUndefined();
    expect(pdfPlace({ viewState: {}, cursorState: [] })).toBeUndefined();
    const place = { page: 3, zoom: 'auto', left: 0, top: 10 };
    expect(pdfPlace({ ...place, page: 0 })).toBeUndefined();
    expect(pdfPlace({ ...place, page: 1.5 })).toBeUndefined();
    expect(pdfPlace({ ...place, zoom: 'huge' })).toBeUndefined();
    expect(pdfPlace({ ...place, zoom: 0 })).toBeUndefined();
    expect(pdfPlace({ ...place, top: null })).toBeUndefined();
  });
});
