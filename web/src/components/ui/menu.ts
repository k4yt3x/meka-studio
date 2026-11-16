import type * as DropdownMenu from '@radix-ui/react-dropdown-menu';

/** The parts menu items are made of, a dropdown menu's or a context menu's. */
export type MenuParts = Pick<typeof DropdownMenu, 'Item' | 'Separator'>;
