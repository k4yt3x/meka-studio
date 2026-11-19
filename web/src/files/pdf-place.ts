/** How a PDF's zoom follows the room it has, as PDF.js names it. */
export type Fit = 'auto' | 'page-width' | 'page-fit' | 'page-actual';

export const FITS: readonly Fit[] = ['auto', 'page-width', 'page-fit', 'page-actual'];

export function isFit(value: unknown): value is Fit {
  return FITS.includes(value as Fit);
}

/**
 * Where a PDF was read to, as kept to open it there again: the page, the zoom as a fit or a
 * percentage, and the point of the page at the view's top left, in the PDF's own units.
 */
export interface PdfPlace {
  page: number;
  zoom: Fit | number;
  left: number;
  top: number;
}

/** `kept` as a {@link PdfPlace}, if it is one: what a browser kept may be of an older build. */
export function pdfPlace(kept: unknown): PdfPlace | undefined {
  if (typeof kept !== 'object' || kept === null) return undefined;
  const { page, zoom, left, top } = kept as Record<string, unknown>;
  if (
    typeof page !== 'number' ||
    !Number.isInteger(page) ||
    page < 1 ||
    !(isFit(zoom) || (typeof zoom === 'number' && zoom > 0)) ||
    typeof left !== 'number' ||
    !Number.isFinite(left) ||
    typeof top !== 'number' ||
    !Number.isFinite(top)
  )
    return undefined;
  return { page, zoom, left, top };
}
