import { expect, it } from 'vitest';
import { minimumVersion, supportsVersion } from './version';

it('reads the shared minimum', () => expect(minimumVersion).toBe('0.95.0'));
it.each(['0.95.0', '0.95.1', '0.100.0', '1.0.0', '0.95.0+build.1'])(
  'connects to Meka %s',
  (version) => expect(supportsVersion(version)).toBe(true),
);
it.each(['', '0.59.0', '0.85.0', '0.94.0', '0.95.0-rc.1', 'unknown', '0.95', '0.95.0extra'])(
  'refuses older or unrecognized servers: %s',
  (version) => expect(supportsVersion(version)).toBe(false),
);
