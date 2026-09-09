import { readFileSync } from 'node:fs'
import path from 'node:path'

import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const fullPage = process.env.TT_SOAK_PAGE === 'full'
const scenario = process.env.TT_SOAK_SCENARIO ?? 'normal'
if (!['normal', 'slow_recovery', 'driver_stall', 'js_stall'].includes(scenario))
  throw new Error('Unknown soak scenario')
const fixture = path.resolve('tests/ui-soak/full-fixtures.ts')
const names = [
  ...readFileSync('src/services/cmds.ts', 'utf8').matchAll(
    /export\s+(?:async\s+)?(?:function|const)\s+(\w+)/g,
  ),
].map((match) => match[1])
export default defineConfig({
  root: path.resolve('tests/ui-soak'),
  plugins: [
    react(),
    ...(fullPage
      ? [
          {
            name: 'isolated-command-boundary',
            resolveId(id: string) {
              if (id === '@/services/cmds') return '\0soak-commands'
            },
            load(id: string) {
              // Vite's @ alias may resolve before this plugin's resolveId hook.
              if (
                id === '\0soak-commands' ||
                id.endsWith('/src/services/cmds.ts')
              )
                return (
                  `import { command } from ${JSON.stringify(fixture)};\n` +
                  names
                    .map(
                      (name) =>
                        `export const ${name} = (...args) => command(${JSON.stringify(name)}, args);`,
                    )
                    .join('\n')
                )
            },
            transformIndexHtml: {
              order: 'pre' as const,
              handler(html: string) {
                return html.replace('/main.tsx', '/full-page.tsx')
              },
            },
            generateBundle(
              _options: unknown,
              bundle: Record<string, { type: string; code?: string }>,
            ) {
              const code = Object.values(bundle)
                .map((item) => item.code ?? '')
                .join('\n')
              if (
                !code.includes('ISOLATED_COMMAND_DENIED') ||
                code.includes('tt_capture_start') ||
                code.includes('tt_get_environment')
              ) {
                throw new Error(
                  'Isolated command boundary was not applied; refusing full-page build',
                )
              }
            },
          },
        ]
      : []),
  ],
  resolve: {
    alias: {
      ...(fullPage
        ? {
            '@/hooks/use-current-proxy': path.resolve(
              'tests/ui-soak/profile-boundary.ts',
            ),
            '@/hooks/use-profiles': path.resolve(
              'tests/ui-soak/profile-boundary.ts',
            ),
            '@/components/base': path.resolve('tests/ui-soak/base-boundary.ts'),
            '@tauri-apps/plugin-dialog': path.resolve(
              'tests/ui-soak/dialog-boundary.ts',
            ),
            '@tauri-apps/api/path': path.resolve(
              'tests/ui-soak/path-boundary.ts',
            ),
            '@tauri-apps/api/event': path.resolve(
              'tests/ui-soak/event-boundary.ts',
            ),
          }
        : {}),
      '@': path.resolve('src'),
    },
  },
  define: {
    OS_PLATFORM: JSON.stringify('linux'),
    __TT_SOAK_SCENARIO__: JSON.stringify(scenario),
  },
  build: { outDir: 'dist', emptyOutDir: true },
})
