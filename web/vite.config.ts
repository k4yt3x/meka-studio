import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { createReadStream, readdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import type { Plugin } from 'vite';
import { defineConfig } from 'vitest/config';

const { version }: { version: string } = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
);

// The dev server passes Meka Studio's API and Meka's to a running Meka Studio, whose public_origin
// is the dev server's address, so its cookie and Origin check work as they do in production.
const gateway = process.env.MEKA_STUDIO_GATEWAY ?? 'http://127.0.0.1:4180';

const pdfjs = new URL(
  './',
  pathToFileURL(createRequire(import.meta.url).resolve('pdfjs-dist/package.json')),
);
const pdfjsVersion: string = JSON.parse(
  readFileSync(new URL('package.json', pdfjs), 'utf8'),
).version;
// Named for the version, the files are kept by browsers as long as the rest of the assets.
const pdfjsFolder = `assets/pdfjs-${pdfjsVersion}/`;

/**
 * The files PDF.js fetches by name rather than imports, served by the dev server and written into
 * the build: the character maps PDFs in Chinese, Japanese and Korean name, the fonts for PDFs that
 * leave out their own, and its image decoders built to JavaScript, which it takes where the page's
 * policy refuses WebAssembly.
 */
function pdfjsFiles(): Plugin {
  const files = () => [
    ...readdirSync(new URL('cmaps/', pdfjs))
      .filter((name) => name.endsWith('.bcmap'))
      .map((name) => `cmaps/${name}`),
    ...readdirSync(new URL('standard_fonts/', pdfjs))
      .filter((name) => /\.(pfb|ttf)$/.test(name))
      .map((name) => `standard_fonts/${name}`),
    'wasm/jbig2_nowasm_fallback.js',
    'wasm/openjpeg_nowasm_fallback.js',
  ];
  return {
    name: 'pdfjs-files',
    configureServer(server) {
      const served = new Set(files());
      server.middlewares.use(`/${pdfjsFolder}`, (request, response, next) => {
        // The names need no escaping, so the path is matched as it was asked for.
        const file = (request.url ?? '').split('?', 1)[0]!.slice(1);
        if (!served.has(file)) return next();
        response.setHeader(
          'Content-Type',
          file.endsWith('.js') ? 'text/javascript' : 'application/octet-stream',
        );
        createReadStream(new URL(file, pdfjs)).pipe(response);
      });
    },
    generateBundle() {
      for (const file of files())
        this.emitFile({
          type: 'asset',
          fileName: pdfjsFolder + file,
          source: readFileSync(new URL(file, pdfjs)),
        });
    },
  };
}

export default defineConfig({
  base: '/',
  define: {
    __MEKA_STUDIO_VERSION__: JSON.stringify(version),
    __PDFJS_FILES__: JSON.stringify(`/${pdfjsFolder}`),
  },
  plugins: [react(), tailwindcss(), pdfjsFiles()],
  server: {
    // Editors and generators can replace a file in several writes; do not serve a partial module.
    watch: { awaitWriteFinish: { stabilityThreshold: 100, pollInterval: 20 } },
    proxy: {
      '/api': { target: gateway, ws: true },
      '/meka': { target: gateway },
    },
  },
  test: {
    include: ['src/**/*.test.{ts,tsx}'],
    environment: 'node',
    clearMocks: true,
    restoreMocks: true,
  },
});
