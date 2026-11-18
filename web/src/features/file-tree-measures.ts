import type { CSSProperties } from 'react';

/**
 * Rows this tall, in pixels, at the default font size, so where each is follows from its place;
 * on a touch screen they are at least as tall as a finger needs.
 */
const ROW_HEIGHT = 22;
const TOUCH_ROW_HEIGHT = 36;
/** The font size the tree's measures are given at, which others scale. */
const BASE_FONT_SIZE = 13;
/** An icon's size, and how far each level is indented, at the default font size. */
const ICON_SIZE = 16;
const INDENT = 14;

/**
 * How tall a tree's rows are at `fontSize`, and the measures its rows are drawn with, as the
 * style that sets them: rows drawn with the tree's styles take its size wherever they are.
 */
export function treeMeasures(fontSize: number, coarse: boolean) {
  // Measured in whole pixels, so the guides between levels stay sharp.
  const scaled = (pixels: number) => Math.round((pixels * fontSize) / BASE_FONT_SIZE);
  const rowHeight = Math.max(coarse ? TOUCH_ROW_HEIGHT : 0, scaled(ROW_HEIGHT));
  return {
    rowHeight,
    style: {
      '--row-height': `${rowHeight}px`,
      '--icon': `${scaled(ICON_SIZE)}px`,
      '--indent': `${scaled(INDENT)}px`,
      fontSize: `${fontSize}px`,
    } as CSSProperties,
  };
}
